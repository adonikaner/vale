//! The little round map's picture, drawn from the tile it is of.
//!
//! ## Why an editor has to make one at all
//!
//! The minimap is not derived from anything at run time. It is one 256x256 BLP
//! per ADT that Blizzard's tools rendered once and shipped, keyed by an MD5 in
//! `textures\Minimap\md5translate.trs` — see
//! [`vale_assets::tables::minimap`], where that lookup is written up. So a
//! tile somebody raises a hill on keeps the picture of the ground that used to
//! be there, and a tile somebody *makes* has no picture at all and draws black.
//! Neither reports anything.
//!
//! ## What is drawn, and what is deliberately not
//!
//! At 256 pixels across a 533-yard tile, one pixel is 2.08 yards and one map
//! chunk is exactly sixteen pixels. Nothing at that scale resolves a texture's
//! own pattern — a ground texture repeats eight times across a chunk, so a
//! repeat is two pixels — which is why this samples a **swatch** per texture
//! rather than the texture: the average of a tileset over a few texels is what
//! survives to this size, and sampling the real thing would be noise.
//!
//! What does survive, and what the picture is actually made of:
//!
//! * **the blend**, layer over layer through `MCAL`, which is what puts a road
//!   on a field;
//! * **the light**, from `MCNR` against the baked sun, which is the whole of
//!   why a minimap reads as terrain rather than as a colour chart — a hillside
//!   is visible as a hillside. **Measured off the shipped pictures**, and it
//!   is a plain ambient-plus-diffuse sun that saturates — see
//!   [`Look::ambient`];
//! * **the baked shadow**, `MCSH`, carried through the same reading — and
//!   **measured at a multiplier of one**, so it changes nothing; see
//!   [`Look::shadow`] for the numbers;
//! * **the water**, from `MCLQ`'s own per-tile flags, in the colour
//!   `Light.dbc` gives that liquid *at that place*, its shallow colour mixed
//!   toward its deep one by the file's own depth byte — see
//!   [`Sources::water_of`], where the measurement is;
//! * **and what stands on it** — every `MDDF` doodad as a footprint, and every
//!   `MODF` building **drawn from above**: its rendered triangles through the
//!   placement, the highest surface winning, each in the mean colour of its
//!   texture. A disc was tried first and is what a harbour looked like before
//!   this: one grey circle the width of the whole root box, over the piers, the
//!   water and the houses alike.
//!
//! **And from the neighbouring tiles as well as this one.** A placement is a
//! record in the file of the tile its origin stands on, and a building's roof
//! does not stop at the seam; drawn from one tile's own file, the half of
//! Menethil that stands on the next tile was missing from the next tile's
//! picture. [`render`] takes the neighbours and draws whatever of theirs lands
//! inside the frame. See `vale_assets::world::collision::hulls_near`, which
//! is the same rule for the shadow bake.
//!
//! ## The last of those is most of what a minimap looks like
//!
//! The first draft drew only the ground and came out as blurry olive mush that
//! resembled the shipped picture beside it in colour and in nothing else.
//! Reported from the window, twice. The reason is that a minimap's whole
//! *texture* — the thing that makes Elwynn read as a forest with a road through
//! it — is the **canopies**, not the tileset: at 2.08 yards a pixel a ground
//! texture's eight repeats across a chunk are two pixels each and average to a
//! flat wash, which is what it drew. A tree is four or five pixels of dark
//! green, and a thousand of them are the picture.
//!
//! **How dark those pixels are is measured, and it is not very**: a fully
//! covered pixel reads 0.95 of the lit ground on Elwynn's forest tile and 1.00
//! on the harbour's, so the tint is a few percent and the picture's texture is
//! mostly the *shape* of the copses. See [`Look::canopy_mix`], and
//! [`analyse`], which is the instrument every number in [`Look`] was set with.
//!
//! So the placements are drawn, and their sizes come from the caller for the
//! reason the swatches do: a model's extent is in its `M2`, which is in an
//! archive this crate does not open. `vale_ide` measures it off
//! `M2::positions` — the **drawn** vertices, not the declared bounding box,
//! which includes animation swing and for a tree is about three times the
//! canopy. That is the same measurement `vale sun` makes for the same reason.
//!
//! ## Where the pictures come from, and why not from here
//!
//! This crate opens no archives — see the crate root — so a texture's swatch is
//! the caller's to supply, the same way [`crate::shadow`] takes its occluder.
//! `vale_ide` builds them by decoding each tileset's smallest mip, which
//! is the same thing its texture picker already does for thumbnails.

use std::collections::HashSet;
use std::sync::Arc;

use vale_assets::tables::light::{LiquidLight, SUN_TOWARD};
use vale_assets::world::adt::{Adt, Mcnk, PlacedModel, ALPHA_SIDE, CHUNKS_PER_SIDE, TILE_SIZE};
use vale_assets::world::collision::transform_point;
use vale_assets::world::wmo::{mopy_flags, Liquid, WmoGroup, WmoRoot};

/// How many pixels a minimap tile is on a side.
///
/// 256, measured off the shipped files rather than chosen: Azeroth's tile
/// 32, 48 is 256x256, DXT1, one level, no alpha.
pub const SIDE: usize = 256;

/// …so one map chunk is this many pixels.
pub const PIXELS_PER_CHUNK: usize = SIDE / CHUNKS_PER_SIDE;

/// A texture, reduced to what survives at two yards a pixel.
///
/// Square, RGB, and small — eight or sixteen a side is plenty. A one-pixel
/// swatch is a legitimate and useful case: it is the texture's average colour,
/// and at this scale it is most of what the picture is made of anyway.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Swatch {
    pub side: usize,
    /// `side * side` RGB triples, row-major.
    pub rgb: Vec<[u8; 3]>,
}

impl Swatch {
    /// One colour, everywhere.
    pub fn flat(rgb: [u8; 3]) -> Swatch {
        Swatch {
            side: 1,
            rgb: vec![rgb],
        }
    }

    /// A swatch from decoded RGBA, box-filtered down to `side`.
    ///
    /// Alpha is dropped: a ground texture has none that means anything, and the
    /// picture this ends up in has none at all.
    pub fn from_rgba(rgba: &[u8], width: usize, height: usize, side: usize) -> Option<Swatch> {
        if width == 0 || height == 0 || side == 0 || rgba.len() < width * height * 4 {
            return None;
        }
        let mut rgb = Vec::with_capacity(side * side);
        for y in 0..side {
            for x in 0..side {
                let (x0, x1) = (x * width / side, ((x + 1) * width / side).max(x * width / side + 1));
                let (y0, y1) = (y * height / side, ((y + 1) * height / side).max(y * height / side + 1));
                let mut sum = [0u32; 3];
                let mut taps = 0u32;
                for sy in y0..y1.min(height) {
                    for sx in x0..x1.min(width) {
                        let at = (sy * width + sx) * 4;
                        for c in 0..3 {
                            sum[c] += u32::from(rgba[at + c]);
                        }
                        taps += 1;
                    }
                }
                let taps = taps.max(1);
                rgb.push([
                    (sum[0] / taps) as u8,
                    (sum[1] / taps) as u8,
                    (sum[2] / taps) as u8,
                ]);
            }
        }
        Some(Swatch { side, rgb })
    }

    /// Sample at a wrapped `(u, v)` in 0..1.
    fn at(&self, u: f32, v: f32) -> [u8; 3] {
        if self.rgb.is_empty() {
            return [0, 0, 0];
        }
        let wrap = |t: f32| {
            let t = t - t.floor();
            ((t * self.side as f32) as usize).min(self.side - 1)
        };
        self.rgb[wrap(v) * self.side + wrap(u)]
    }
}

/// What the picture is made of beyond the textures themselves.
#[derive(Debug, Clone, Copy)]
pub struct Look {
    /// **The light the shipped pictures were rendered under**, as the two
    /// terms of a fixed-function sun: the ground is multiplied by
    /// `ambient + diffuse * (normal . sun)`, clamped to one — see [`lit`].
    ///
    /// **Measured, and the same on every tile tried.** `vale bake <Map>
    /// <x> <y> minimap` bins the dry, unshadowed, uncovered pixels by their
    /// `MCNR` normal's dot with the baked sun and divides the shipped
    /// picture's mean by the unlit blend's in each bin. Over seven tiles on
    /// two continents — Elwynn, Duskwood, three in the Wetlands, Aerie Peak,
    /// the Burning Steppes — the ratio reads 0.54 at a dot of 0.1, 0.67 at
    /// 0.25, 0.75 at 0.35, 0.83 at 0.45, 0.90 at 0.55, 0.95 at 0.65 and 1.00
    /// from 0.75 up, within two hundredths on every tile. That is a line of
    /// slope 0.78 through 0.46, saturating at one. The earlier term here ran
    /// linearly from 0.25 to 1.75 about a dot of 0.6, which drew every
    /// sun-facing hillside at half again its texture and was reported as the
    /// cliff tile drawing brighter than the flat one beside it.
    ///
    /// That the curve is tight in *this* dot is also the third confirmation
    /// of [`SUN_TOWARD`]: a bake under another sun would smear it.
    pub ambient: f32,
    pub diffuse: f32,
    /// What a texel in `MCSH` multiplies the colour by.
    ///
    /// **Measured** the same way, as the residual after the slope term, and
    /// it is **one**: where `MCSH` is set the shipped picture is 0.97 to 1.01
    /// of the lit ground on every tile that carries a shadow — Elwynn's
    /// 15,066 shadowed pixels, Duskwood's 16,387, the harbour's 2,670. Raw,
    /// the shadowed pixels read 0.88 of the unlit blend against 0.84 for the
    /// unshadowed, which is the forest floor being flatter than the hills
    /// and not the shadow doing anything. The shipped pictures were rendered
    /// without the baked shadow, or with the canopy that cast it drawn over
    /// it, and either way this multiplies by one. It was 0.65 — the world's
    /// `terrain1.bls` darkens the sun term by `shadow * 0.3 + 0.7` — and that
    /// drew every forest a third darker than shipped. The sweep agrees:
    /// `r` rises monotonically toward 1.0 on all three shadowed tiles.
    pub shadow: f32,
    /// What a doodad's footprint is tinted toward, and how far.
    ///
    /// Dark, because from above almost everything that stands on the ground is
    /// darker than the ground: a canopy, a roof, a rock's shaded side. Tinting
    /// rather than replacing keeps the ground's own colour showing through, so
    /// a tree on grass and the same tree on sand are not the same green.
    pub canopy: [u8; 3],
    pub canopy_mix: f32,
}

impl Default for Look {
    fn default() -> Self {
        Look {
            ambient: 0.46,
            diffuse: 0.78,
            shadow: 1.0,
            canopy: [34, 48, 28],
            // **Measured as a residual, like the shadow.** Pixels the canopy
            // buffer marks as fully covered read 0.95 of the lit ground on
            // Elwynn's forest tile, 1.00 on the harbour's and 1.01 on
            // Duskwood's, so a tree from above is a few percent darker than
            // the ground and no more. At 0.30 this drew a copse at 0.76 of
            // the ground — a quarter too dark — and was the larger half of
            // why a forest tile came out darker than shipped. A tenth of the
            // way toward the canopy colour is a 5% darkening on grass, which
            // is the measured number; the sweep still prefers zero, for the
            // reason it always did — the statistic cannot match a blob texel
            // for texel — and zero is the flat wash this exists to fix.
            canopy_mix: 0.10,
        }
    }
}

/// **A building's drawn surface**, model-space, each triangle in the mean
/// colour of its texture. Built once per WMO path by [`wmo_shape`] and shared
/// between every placement of it.
#[derive(Debug, Clone, Default)]
pub struct Shape {
    pub triangles: Vec<Triangle>,
}

#[derive(Debug, Clone, Copy)]
pub struct Triangle {
    pub corners: [[f32; 3]; 3],
    pub colour: [u8; 3],
}

/// **A WMO reduced to what a picture from above needs**: its rendered
/// triangles and one colour each.
///
/// `colour_of` answers a texture path with the texture's mean colour, which the
/// caller has because the caller has the archives — the same arrangement as the
/// swatches. A texture it cannot resolve draws mid grey, for the reason an
/// unresolved tileset does: a hole is a wrong shape and a grey is only a wrong
/// colour.
///
/// **Only `RENDER` triangles**, which is the flag the client draws by. A
/// collision-only triangle (`mopy_flags::COLLISION` alone) is an invisible
/// hull and would draw a wall nobody can see; a `DETAIL` one is trim laid over
/// a rendered surface and is rendered too.
pub fn wmo_shape(
    root: &WmoRoot,
    groups: &[WmoGroup],
    mut colour_of: impl FnMut(&str) -> Option<[u8; 3]>,
) -> Shape {
    const UNKNOWN: [u8; 3] = [110, 110, 110];
    let mut triangles = Vec::new();
    // One colour per material, resolved once: a texture is named by many
    // batches across many groups.
    let colours: Vec<[u8; 3]> = root
        .materials
        .iter()
        .map(|m| {
            m.texture
                .and_then(|t| root.textures.get(t as usize))
                .and_then(|path| colour_of(path))
                .unwrap_or(UNKNOWN)
        })
        .collect();
    for group in groups {
        for batch in &group.batches {
            let colour = batch
                .material
                .and_then(|m| colours.get(m as usize).copied())
                .unwrap_or(UNKNOWN);
            let first = batch.index_start as usize / 3;
            let count = batch.index_count as usize / 3;
            for t in first..first + count {
                let drawn = group
                    .triangle_flags
                    .get(t)
                    .is_some_and(|&f| f & mopy_flags::RENDER != 0);
                if !drawn {
                    continue;
                }
                let corner = |k: usize| -> Option<[f32; 3]> {
                    let index = *group.indices.get(t * 3 + k)? as usize;
                    group.positions.get(index).copied()
                };
                let (Some(a), Some(b), Some(c)) = (corner(0), corner(1), corner(2)) else {
                    continue;
                };
                triangles.push(Triangle {
                    corners: [a, b, c],
                    colour,
                });
            }
        }
    }
    Shape { triangles }
}

/// **Everything a picture needs that is not in the tile**, each supplied by
/// the caller because each comes out of an archive this crate does not open.
pub struct Sources<'a> {
    /// A ground texture, reduced to a swatch.
    pub swatch_of: &'a dyn Fn(&str) -> Option<Swatch>,
    /// How wide a doodad's canopy is — measured off its drawn vertices, not its
    /// declared box; see the module comment. `None` skips the placement.
    pub radius_of: &'a dyn Fn(&str) -> Option<f32>,
    /// A building's drawn surface, from [`wmo_shape`]. `None` skips it.
    pub shape_of: &'a dyn Fn(&str) -> Option<Arc<Shape>>,
    /// **What colour a liquid is here**, from `Light.dbc` at the tile's own
    /// centre, at noon — `DisplayTables::liquid_depth_light`: the zone's
    /// light, read as a **shallow** colour in `close` and a **deep** one in
    /// `far`, with the float bands' two opacities.
    ///
    /// **Measured, twice.** The shipped pictures were rendered through the
    /// game's own light chain, and read back where each tile's `MCLQ` says
    /// the water is, their water is the zone's colour and not a blue: a fixed
    /// blue drew every lake in the game the same colour and the Wetlands'
    /// green sea bright blue. The second measurement is *which* rows, binned
    /// by the depth byte at each pixel: the ocean at Menethil reads band 14
    /// of the Wetlands light over the sea floor, the open sea at byte 255
    /// reads band 15 of the map's light exactly (`0/29/41`), and the bytes
    /// between read the mix of the two; Elwynn's lake does the same one pair
    /// up, bands 16 and 17. Those are one row above the four the world's
    /// surfaces are drawn with — `vale_assets::tables::light::band` has
    /// the numbers — and are read through the accessor named above so that
    /// the minimap is right without deciding the client's water for it.
    ///
    /// `None` for a liquid the chain has no light for. Magma and slime are
    /// that by design (their own texture carries the colour) and draw in the
    /// texture's measured mean; water and ocean with no `Light.dbc` at all fall
    /// back to a blue rather than to the chain's white.
    pub water_of: &'a dyn Fn(Liquid) -> Option<LiquidLight>,
}

/// What a liquid draws as when the caller has no light for it — see
/// [`Sources::water_of`]. The magma and slime numbers are the mean colour of
/// `lava.1.blp` and `slime.1.blp`, which `vale water` prints.
fn unlit(kind: Liquid) -> LiquidLight {
    let rgb = |r: u8, g: u8, b: u8| {
        [
            f32::from(r) / 255.0,
            f32::from(g) / 255.0,
            f32::from(b) / 255.0,
        ]
    };
    match kind {
        Liquid::Magma => LiquidLight {
            close: rgb(175, 22, 0),
            far: rgb(175, 22, 0),
            shallow_alpha: 1.0,
            deep_alpha: 1.0,
        },
        Liquid::Slime => LiquidLight {
            close: rgb(68, 132, 18),
            far: rgb(68, 132, 18),
            shallow_alpha: 1.0,
            deep_alpha: 1.0,
        },
        Liquid::Water | Liquid::Ocean => LiquidLight {
            close: rgb(86, 132, 170),
            far: rgb(26, 52, 92),
            shallow_alpha: 0.55,
            deep_alpha: 1.0,
        },
    }
}

/// **Draw a tile.** Returns `SIDE * SIDE` RGBA, opaque throughout.
///
/// Takes a parsed [`Adt`] rather than this crate's own [`crate::adt::AdtFile`]
/// on purpose: what a picture needs is every rule `vale-assets` already
/// applies — the alpha maps decoded and stretched, the shadow unpacked, the
/// normals decoded, the liquid re-ordered into its own frame — and a second
/// reading of any of them here would be a second thing to keep true. An editor
/// holding an `AdtFile` writes it and parses it, which costs a couple of
/// milliseconds against a hundred for the picture.
///
/// `neighbours` are the tiles around it, for their placements only; the ground
/// drawn is `adt`'s. Pass none and a building on the next tile ends at the seam.
///
/// ## The orientation is the part that can be plausibly wrong
///
/// Pixel row runs along **decreasing world x** and pixel column along
/// **decreasing world y**, from the tile's north-west corner — which is the
/// same frame the chunk texels and the chunk indices use, and the one
/// `vale minimap <Map>` votes for over twenty-five tiles by correlating each
/// of the four readings of a shipped picture against that tile's own `MCLQ`. A
/// picture written in any other reading is a map that is mirrored or turned,
/// draws perfectly, and is wrong everywhere the coastline is not symmetric.
pub fn render(adt: &Adt, neighbours: &[Adt], sources: &Sources, look: &Look) -> Vec<u8> {
    let swatches: Vec<Option<Swatch>> = adt
        .texture_names
        .iter()
        .map(|name| (sources.swatch_of)(name))
        .collect();
    // A texture the caller cannot resolve draws as mid grey rather than as
    // nothing: a hole in the picture reads as water, which is worse than a
    // wrong colour because it is a wrong *shape*.
    let unknown = Swatch::flat([110, 110, 110]);
    let sun = normalise(SUN_TOWARD);
    // The lights, resolved once each: a tile is one place.
    let water: Vec<LiquidLight> = Liquid::ALL
        .iter()
        .map(|&kind| (sources.water_of)(kind).unwrap_or_else(|| unlit(kind)))
        .collect();

    // **What stands on the ground, rasterised once.** Per pixel this would be a
    // thousand placements tested against 65,536 texels; as a coverage buffer it
    // is one pass over the placements and one lookup a pixel.
    let canopy = canopies(adt, neighbours, sources.radius_of);
    let roof = roofs(adt, neighbours, sources.shape_of, sun, look);

    let mut out = vec![255u8; SIDE * SIDE * 4];
    for py in 0..SIDE {
        for px in 0..SIDE {
            let cell = (py / PIXELS_PER_CHUNK) * CHUNKS_PER_SIDE + px / PIXELS_PER_CHUNK;
            let Some(chunk) = adt.chunks.get(cell) else {
                continue;
            };
            // Where in the chunk, 0..1 in its own two decreasing axes.
            let fx = (px % PIXELS_PER_CHUNK) as f32 / PIXELS_PER_CHUNK as f32;
            let fy = (py % PIXELS_PER_CHUNK) as f32 / PIXELS_PER_CHUNK as f32;
            let at_pixel = py * SIDE + px;
            let mut colour = match roof[at_pixel] {
                // A roof hides everything under it, the water included: a pier
                // is over the harbour, not under it.
                Some(roof) => roof,
                None => {
                    let ground = sample(chunk, &swatches, &unknown, fx, fy, sun);
                    finish(&ground, &water, look)
                }
            };
            // …and the canopies over that, so a tree beside a house shades its
            // eave the way it does on the ground.
            let cover = canopy[at_pixel];
            if cover > 0.0 {
                let a = (cover * look.canopy_mix).clamp(0.0, 1.0);
                for (channel, tint) in colour.iter_mut().zip(look.canopy) {
                    let was = f32::from(*channel);
                    *channel = (was + (f32::from(tint) - was) * a) as u8;
                }
            }
            let at = at_pixel * 4;
            out[at..at + 3].copy_from_slice(&colour);
        }
    }
    out
}

/// **What every pixel of a tile is made of, before the look is applied.**
///
/// The same reading [`render`] paints from — the same [`sample`] at the same
/// texel, the same canopy and roof buffers — kept apart so that a picture can
/// be measured against the shipped one *by what is under each pixel*: the
/// slope, whether it is in the baked shadow, how much canopy covers it, which
/// texture it is, how deep the water is. That is how every number in
/// [`Look`] was set, and it is what `vale bake <Map> <x> <y> minimap`
/// prints. A correlation over the whole picture says which of two looks is
/// closer and nothing about what is wrong with either.
///
/// Every field is `SIDE * SIDE`, row-major, in the picture's own frame.
#[derive(Debug, Clone)]
pub struct Analysis {
    /// The blended texture colour, unlit: the swatches through `MCAL` and
    /// nothing else. What the shipped picture is divided by to read a gain.
    pub unlit: Vec<[f32; 3]>,
    /// The `MCNR` normal's dot with the baked sun, `1.0` where there is no
    /// chunk.
    pub facing: Vec<f32>,
    /// Whether `MCSH` shadows the texel.
    pub shadow: Vec<bool>,
    /// Canopy coverage, 0 to 1 — see [`canopies`].
    pub canopy: Vec<f32>,
    /// Whether a building's surface is drawn over the pixel.
    pub roof: Vec<bool>,
    /// The texture id (into the tile's `MTEX`) of the layer contributing most
    /// at the texel, or `u32::MAX` where there is no chunk or no layer.
    pub texture: Vec<u32>,
    /// Yards of liquid over the ground, zero where dry.
    pub depth: Vec<f32>,
    /// …and the file's own depth byte there, zero where dry.
    pub depth_byte: Vec<u8>,
}

/// Read a tile the way [`render`] does, without painting it.
pub fn analyse(adt: &Adt, neighbours: &[Adt], sources: &Sources) -> Analysis {
    let swatches: Vec<Option<Swatch>> = adt
        .texture_names
        .iter()
        .map(|name| (sources.swatch_of)(name))
        .collect();
    let unknown = Swatch::flat([110, 110, 110]);
    let sun = normalise(SUN_TOWARD);
    let canopy = canopies(adt, neighbours, sources.radius_of);
    let roofs = roofs(adt, neighbours, sources.shape_of, sun, &Look::default());
    let n = SIDE * SIDE;
    let mut out = Analysis {
        unlit: vec![[0.0; 3]; n],
        facing: vec![1.0; n],
        shadow: vec![false; n],
        canopy,
        roof: roofs.iter().map(Option::is_some).collect(),
        texture: vec![u32::MAX; n],
        depth: vec![0.0; n],
        depth_byte: vec![0; n],
    };
    for py in 0..SIDE {
        for px in 0..SIDE {
            let cell = (py / PIXELS_PER_CHUNK) * CHUNKS_PER_SIDE + px / PIXELS_PER_CHUNK;
            let Some(chunk) = adt.chunks.get(cell) else {
                continue;
            };
            let fx = (px % PIXELS_PER_CHUNK) as f32 / PIXELS_PER_CHUNK as f32;
            let fy = (py % PIXELS_PER_CHUNK) as f32 / PIXELS_PER_CHUNK as f32;
            let at = py * SIDE + px;
            let ground = sample(chunk, &swatches, &unknown, fx, fy, sun);
            out.unlit[at] = ground.colour;
            out.facing[at] = ground.facing;
            out.shadow[at] = ground.shadow;
            out.texture[at] = ground.texture.unwrap_or(u32::MAX);
            if let Some((_, depth, byte)) = ground.water {
                out.depth[at] = depth;
                out.depth_byte[at] = byte;
            }
        }
    }
    out
}

/// The tile's own maximum corner, which is chunk 0's — where pixel `(0, 0)`
/// sits.
fn picture_origin(adt: &Adt) -> Option<[f32; 2]> {
    let first = adt.chunks.first()?;
    Some([first.position[0], first.position[1]])
}

/// **Every placement of one kind by this tile or a neighbour, once each.**
///
/// A model straddling a seam is recorded by both tiles with the same
/// `unique_id`, which is the client's own rule for drawing it once; the same
/// rule keeps a canopy from being laid twice as dark along every seam.
fn placements(
    adt: &Adt,
    neighbours: &[Adt],
    of: impl Fn(&Adt) -> Vec<PlacedModel>,
) -> Vec<PlacedModel> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for tile in std::iter::once(adt).chain(neighbours.iter()) {
        for placed in of(tile) {
            if seen.insert(placed.unique_id) {
                out.push(placed);
            }
        }
    }
    out
}

/// **Where the doodads are**, as a coverage buffer: discs, `SIDE * SIDE`,
/// 0 to 1.
///
/// A placement whose model the caller cannot size is **skipped**, which is the
/// right failure: the alternative is a guessed radius, and a guess that is too
/// large paints a forest over a field.
fn canopies(adt: &Adt, neighbours: &[Adt], radius_of: &dyn Fn(&str) -> Option<f32>) -> Vec<f32> {
    let mut canopy = vec![0f32; SIDE * SIDE];
    let Some(origin) = picture_origin(adt) else {
        return canopy;
    };
    let per_pixel = TILE_SIZE / SIDE as f32;
    for placed in placements(adt, neighbours, Adt::placed_doodads) {
        let Some(radius) = radius_of(&placed.path) else {
            continue;
        };
        disc(
            &mut canopy,
            origin,
            per_pixel,
            [placed.position[0], placed.position[1]],
            radius * placed.scale,
        );
    }
    canopy
}

/// **The buildings, seen from above**: `SIDE * SIDE` colours, `None` where no
/// building stands over the ground.
///
/// Every rendered triangle of every placed WMO, through its placement matrix,
/// into a depth buffer that starts at the ground — so a cellar stays under the
/// hill and a roof stands over it — with the highest surface at each pixel
/// winning. Lit by its own facing against the baked sun, on the ground's own
/// terms, so a pitched roof reads as pitched.
///
/// A wall projects to nothing from above and is skipped by its area; what is
/// left is roofs, floors, piers and stairs, which is what a minimap shows of a
/// building.
fn roofs(
    adt: &Adt,
    neighbours: &[Adt],
    shape_of: &dyn Fn(&str) -> Option<Arc<Shape>>,
    sun: [f32; 3],
    look: &Look,
) -> Vec<Option<[u8; 3]>> {
    let mut roof = vec![None; SIDE * SIDE];
    let Some(origin) = picture_origin(adt) else {
        return roof;
    };
    let per_pixel = TILE_SIZE / SIDE as f32;

    // The ground, as the depth buffer's floor. Where the tile has no chunk the
    // floor is the bottom of the world, so a building over a hole still draws.
    let mut depth = vec![f32::MIN; SIDE * SIDE];
    for py in 0..SIDE {
        for px in 0..SIDE {
            let cell = (py / PIXELS_PER_CHUNK) * CHUNKS_PER_SIDE + px / PIXELS_PER_CHUNK;
            let Some(chunk) = adt.chunks.get(cell) else {
                continue;
            };
            let fx = (px % PIXELS_PER_CHUNK) as f32 / PIXELS_PER_CHUNK as f32;
            let fy = (py % PIXELS_PER_CHUNK) as f32 / PIXELS_PER_CHUNK as f32;
            if let Some(z) = ground_height(chunk, fx, fy) {
                depth[py * SIDE + px] = z;
            }
        }
    }

    for placed in placements(adt, neighbours, Adt::placed_wmos) {
        let Some(shape) = shape_of(&placed.path) else {
            continue;
        };
        for triangle in &shape.triangles {
            let world = triangle.corners.map(|c| transform_point(&placed.matrix, c));
            // World to pixel: rows run along decreasing x, columns along
            // decreasing y — the same frame [`disc`] uses.
            let at = |v: [f32; 3]| {
                [
                    (origin[1] - v[1]) / per_pixel,
                    (origin[0] - v[0]) / per_pixel,
                ]
            };
            let p = world.map(at);
            let lit = facing(&world, sun, look);
            let colour = triangle
                .colour
                .map(|c| (f32::from(c) * lit).clamp(0.0, 255.0) as u8);
            raster(&p, &world.map(|v| v[2]), &mut |index, z| {
                if z > depth[index] {
                    depth[index] = z;
                    roof[index] = Some(colour);
                }
            });
        }
    }
    roof
}

/// A triangle's lit term from its own normal, the ground's own rule: see
/// [`pixel`]. Seen from above, so a normal pointing down is turned up first —
/// a floor is drawn by the side that faces the sky whichever way the file
/// wound it.
fn facing(world: &[[f32; 3]; 3], sun: [f32; 3], look: &Look) -> f32 {
    let e1 = [
        world[1][0] - world[0][0],
        world[1][1] - world[0][1],
        world[1][2] - world[0][2],
    ];
    let e2 = [
        world[2][0] - world[0][0],
        world[2][1] - world[0][1],
        world[2][2] - world[0][2],
    ];
    let mut n = normalise([
        e1[1] * e2[2] - e1[2] * e2[1],
        e1[2] * e2[0] - e1[0] * e2[2],
        e1[0] * e2[1] - e1[1] * e2[0],
    ]);
    if n[2] < 0.0 {
        n = [-n[0], -n[1], -n[2]];
    }
    let dot = (n[0] * sun[0] + n[1] * sun[1] + n[2] * sun[2]).clamp(-1.0, 1.0);
    lit(dot, look)
}

/// **The lit term for a surface facing the sun by `facing`** — the dot of its
/// normal with [`SUN_TOWARD`] — under the look's light: ambient plus diffuse
/// times the facing, clamped to one. See [`Look::ambient`], where the
/// measurement is. A surface facing away takes the ambient alone.
pub fn lit(facing: f32, look: &Look) -> f32 {
    (look.ambient + look.diffuse * facing.max(0.0)).clamp(0.0, 1.0)
}

/// **Fill a triangle's pixels**, calling `hit` with each pixel's index and the
/// surface's height there.
///
/// Edge functions over the pixel centres inside the triangle's box, the height
/// interpolated barycentrically. A triangle with no area from above — a wall —
/// hits nothing.
fn raster(p: &[[f32; 2]; 3], z: &[f32; 3], hit: &mut dyn FnMut(usize, f32)) {
    let area = (p[1][0] - p[0][0]) * (p[2][1] - p[0][1])
        - (p[2][0] - p[0][0]) * (p[1][1] - p[0][1]);
    if area.abs() < 1.0e-6 {
        return;
    }
    let lo = |k: usize| {
        p.iter()
            .map(|v| v[k])
            .fold(f32::MAX, f32::min)
            .floor()
            .clamp(0.0, SIDE as f32) as usize
    };
    let hi = |k: usize| {
        p.iter()
            .map(|v| v[k])
            .fold(f32::MIN, f32::max)
            .ceil()
            .clamp(0.0, SIDE as f32) as usize
    };
    let (x0, x1, y0, y1) = (lo(0), hi(0), lo(1), hi(1));
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    let edge = |a: [f32; 2], b: [f32; 2], x: f32, y: f32| {
        (b[0] - a[0]) * (y - a[1]) - (b[1] - a[1]) * (x - a[0])
    };
    for py in y0..y1 {
        for px in x0..x1 {
            let (x, y) = (px as f32 + 0.5, py as f32 + 0.5);
            let w0 = edge(p[1], p[2], x, y) / area;
            let w1 = edge(p[2], p[0], x, y) / area;
            let w2 = edge(p[0], p[1], x, y) / area;
            if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                continue;
            }
            hit(py * SIDE + px, w0 * z[0] + w1 * z[1] + w2 * z[2]);
        }
    }
}

/// Fill a disc of world radius into a coverage buffer.
///
/// **Coverage adds and saturates**, so a copse of overlapping trees is one
/// clump rather than a lattice of darker crossings — which is what a minimap of
/// a forest looks like, and what averaging would not give.
///
/// A doodad smaller than a pixel still marks the pixel it is in, at a coverage
/// of its share of it. Most of the world is those: a tuft of grass is a tenth
/// of a pixel, and a thousand of them are what makes a field read as a field
/// rather than as flat colour.
fn disc(cover: &mut [f32], origin: [f32; 2], per_pixel: f32, at: [f32; 2], radius: f32) {
    // World to pixel: rows run along decreasing x, columns along decreasing y.
    let cx = (origin[1] - at[1]) / per_pixel;
    let cy = (origin[0] - at[0]) / per_pixel;
    let r = (radius / per_pixel).max(0.0);

    // A sub-pixel doodad marks its own pixel and nothing else.
    if r < 0.5 {
        let (px, py) = (cx.floor(), cy.floor());
        if px < 0.0 || py < 0.0 || px >= SIDE as f32 || py >= SIDE as f32 {
            return;
        }
        let at = py as usize * SIDE + px as usize;
        // Its area as a fraction of the pixel's, capped so that a swarm of
        // small things saturates rather than overflowing.
        cover[at] = (cover[at] + (r * r * std::f32::consts::PI).min(1.0)).min(1.0);
        return;
    }

    // **The edge is a gradient, not a step.** A disc that filled every pixel
    // whose *centre* it covered was measurably too heavy: sweeping the radius
    // against two shipped tiles, halving it improved the match on both — which
    // is not a statement that canopies are smaller than they are, but that a
    // hard edge overstates a four-pixel blob by its whole rim. Coverage is the
    // pixel's share of the disc, approximated by the distance to its centre
    // over one pixel of falloff.
    let lo = |v: f32| (v - r - 1.0).floor().max(0.0) as usize;
    let hi = |v: f32| ((v + r + 1.0).ceil().min(SIDE as f32)) as usize;
    for py in lo(cy)..hi(cy) {
        for px in lo(cx)..hi(cx) {
            let dx = px as f32 + 0.5 - cx;
            let dy = py as f32 + 0.5 - cy;
            let share = (r + 0.5 - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0);
            if share <= 0.0 {
                continue;
            }
            let at = py * SIDE + px;
            cover[at] = (cover[at] + share).min(1.0);
        }
    }
}

/// One pixel of one chunk, read but not yet painted: what is there, and the
/// three things the look decides how to show.
struct Ground {
    /// The blended swatches, unlit.
    colour: [f32; 3],
    /// The layer contributing most, by texture id.
    texture: Option<u32>,
    /// The shading normal's dot with the sun.
    facing: f32,
    /// Whether `MCSH` shadows the texel.
    shadow: bool,
    /// The liquid over it: kind, yards of depth, and the file's own depth
    /// byte.
    water: Option<(Liquid, f32, u8)>,
}

/// Read one pixel of one chunk: the ground's own facts, with nothing of the
/// look applied. [`finish`] paints it; [`analyse`] reports it.
fn sample(
    chunk: &Mcnk,
    swatches: &[Option<Swatch>],
    unknown: &Swatch,
    fx: f32,
    fy: f32,
    sun: [f32; 3],
) -> Ground {
    // The alpha grid is 64 across the chunk; the pixel grid is 16. Sample at
    // the middle of the four texels a pixel covers.
    let tx = ((fx * ALPHA_SIDE as f32) as usize).min(ALPHA_SIDE - 1);
    let ty = ((fy * ALPHA_SIDE as f32) as usize).min(ALPHA_SIDE - 1);
    let texel = ty * ALPHA_SIDE + tx;

    // The texture repeats eight times across a chunk, so the swatch does too —
    // which at sixteen pixels a chunk is mostly an average, and is meant to be.
    let (u, v) = (fx * 8.0, fy * 8.0);

    let mut colour = [0f32; 3];
    let mut first = true;
    // Each layer's blend at this texel; the base's is one.
    let mut alphas = Vec::with_capacity(chunk.layers.len());
    for (i, layer) in chunk.layers.iter().enumerate() {
        let swatch = swatches
            .get(layer.texture_id as usize)
            .and_then(|s| s.as_ref())
            .unwrap_or(unknown);
        let sample = swatch.at(u, v);
        if first {
            colour = [f32::from(sample[0]), f32::from(sample[1]), f32::from(sample[2])];
            first = false;
            alphas.push(1.0);
            continue;
        }
        let a = chunk
            .alphas
            .get(i)
            .and_then(|m| m.get(texel))
            .map(|&a| f32::from(a) / 255.0)
            .unwrap_or(0.0);
        for c in 0..3 {
            colour[c] += (f32::from(sample[c]) - colour[c]) * a;
        }
        alphas.push(a);
    }
    if first {
        // A chunk with no layers at all. Rare and legitimate.
        colour = [110.0, 110.0, 110.0];
    }
    // Which layer shows most: a layer's share of the finished pixel is its
    // own blend times what every layer over it left uncovered.
    let mut texture = None;
    let mut strongest = 0.0f32;
    let mut uncovered = 1.0f32;
    for (i, a) in alphas.iter().enumerate().rev() {
        let share = a * uncovered;
        if share > strongest {
            strongest = share;
            texture = Some(chunk.layers[i].texture_id);
        }
        uncovered *= 1.0 - a;
    }

    // `MCNR`'s normals are the shading normals the game itself uses, so this
    // is the same term the world is lit by rather than a geometric slope
    // computed here.
    let n = nearest_normal(chunk, fx, fy);
    let facing = (n[0] * sun[0] + n[1] * sun[1] + n[2] * sun[2]).clamp(-1.0, 1.0);
    let shadow = chunk.shadow.get(texel).copied().unwrap_or(0) != 0;

    Ground {
        colour,
        texture,
        facing,
        shadow,
        water: under_water(chunk, fx, fy),
    }
}

/// Paint one read pixel: lit by its slope, darkened by the baked shadow, and
/// under its water.
fn finish(ground: &Ground, water: &[LiquidLight], look: &Look) -> [u8; 3] {
    let mut colour = ground.colour;

    // The light — see [`lit`], and [`Look::ambient`] for the measurement.
    let lit = lit(ground.facing, look);
    for channel in &mut colour {
        *channel *= lit;
    }

    if ground.shadow {
        for channel in &mut colour {
            *channel *= look.shadow;
        }
    }

    // …and the water on top of all of it, since a lake hides what is under it.
    if let Some((kind, _, byte)) = ground.water {
        // **The file's own depth byte, through the client's own ramp.** The
        // first draft tinted every texel of a liquid *tile* one flat dark
        // blue, which drew the sea as a slab with a hard edge at the shore;
        // the second graded the light's shallow opacity to its deep one over
        // eight yards of measured depth, which drew the Wetlands' sea at 95%
        // where the shipped picture shows a quarter of the sea floor through
        // it at 24 yards down. `MCLQ` carries an opacity byte at every vertex,
        // and it is not the depth: on `Azeroth_33_39` it tops out at 78 of
        // 255 over water 45 yards deep. `LiquidLight::at_depth` is what the
        // world's own surface does with it, and read that way the drawn sea
        // lands within four units of the shipped one in every depth band.
        // The colour is the light's own, and it is **two** colours, the
        // shallow one at byte 0 and the deep one at 255 — see
        // [`Sources::water_of`], where the measurement is.
        let light = &water[kind.index()];
        let alpha = light.at_depth(byte)[3];
        let t = f32::from(byte) / 255.0;
        for (c, channel) in colour.iter_mut().enumerate() {
            let tint = (light.close[c] + (light.far[c] - light.close[c]) * t) * 255.0;
            *channel += (tint - *channel) * alpha.clamp(0.0, 1.0);
        }
    }

    [
        colour[0].clamp(0.0, 255.0) as u8,
        colour[1].clamp(0.0, 255.0) as u8,
        colour[2].clamp(0.0, 255.0) as u8,
    ]
}

/// The normal nearest a point in the chunk, off `MCNR`'s 9x9 outer grid.
///
/// Nearest rather than interpolated: the grid is nine across and the picture is
/// sixteen, so interpolation would smooth away exactly the ridge lines that make
/// the picture readable.
fn nearest_normal(chunk: &Mcnk, fx: f32, fy: f32) -> [f32; 3] {
    let row = ((fy * 8.0).round() as usize).min(8);
    let col = ((fx * 8.0).round() as usize).min(8);
    // The outer grid is rows of 9 with an inner row of 8 between each pair —
    // the 145-vertex layout. Outer vertex (row, col) is at `row * 17 + col`.
    chunk
        .normals
        .get(row * 17 + col)
        .copied()
        .unwrap_or([0.0, 0.0, 1.0])
}

/// **Which liquid this point is under, how far, and what the file says about
/// it**, or `None` where it is dry.
///
/// Two questions and both matter. `MCLQ`'s per-tile flags say which parts of the
/// chunk hold liquid at all — that is what turns a rectangle into a pool — and
/// the grid's own heights say where its surface is. Asking only the first tints
/// a rock standing out of a lake as though it were under it, and at eight liquid
/// tiles across a chunk that rock is two pixels of flat blue in the middle of
/// dry ground.
///
/// The third answer is the **depth byte** the four vertices around the point
/// carry, interpolated — the opacity's own parameter, which [`finish`] reads
/// through the same ramp the world's surface does.
///
/// The liquid grid runs **+X and +Y from the chunk's low corner** after
/// `parse_mclq_block` has re-ordered it, where the texel grid runs the other way
/// from the high corner — so `fx` and `fy` are flipped here and only here.
fn under_water(chunk: &Mcnk, fx: f32, fy: f32) -> Option<(Liquid, f32, u8)> {
    let ground = ground_height(chunk, fx, fy)?;
    chunk
        .liquids
        .iter()
        .filter_map(|(kind, grid)| {
            let (sx, sy) = ((1.0 - fy) * grid.x_tiles as f32, (1.0 - fx) * grid.y_tiles as f32);
            let gx = (sx as usize).min(grid.x_tiles.saturating_sub(1));
            let gy = (sy as usize).min(grid.y_tiles.saturating_sub(1));
            if !grid.has_liquid(gx, gy) {
                return None;
            }
            let surface = grid.height(gx, gy);
            if surface <= ground {
                return None;
            }
            // The byte, bilinear over the cell's four corners.
            let stride = grid.x_tiles + 1;
            let byte_at = |x: usize, y: usize| {
                f32::from(grid.depths.get(y * stride + x).copied().unwrap_or(0))
            };
            let (u, v) = ((sx - gx as f32).clamp(0.0, 1.0), (sy - gy as f32).clamp(0.0, 1.0));
            let byte = byte_at(gx, gy) * (1.0 - u) * (1.0 - v)
                + byte_at(gx + 1, gy) * u * (1.0 - v)
                + byte_at(gx, gy + 1) * (1.0 - u) * v
                + byte_at(gx + 1, gy + 1) * u * v;
            Some((*kind, surface - ground, byte.round().clamp(0.0, 255.0) as u8))
        })
        // The deepest, for a chunk carrying a river and the ocean it flows into.
        .fold(None, |best: Option<(Liquid, f32, u8)>, next| match best {
            Some((_, b, _)) if b >= next.1 => best,
            _ => Some(next),
        })
}

/// The terrain height at a point in the chunk, off `MCVT`'s outer grid.
///
/// Nearest rather than interpolated, for the same reason [`nearest_normal`] is:
/// the picture is sixteen pixels across a chunk and the grid is nine, so the
/// error is below a pixel and interpolating costs three more lookups a texel.
fn ground_height(chunk: &Mcnk, fx: f32, fy: f32) -> Option<f32> {
    let row = ((fy * 8.0).round() as usize).min(8);
    let col = ((fx * 8.0).round() as usize).min(8);
    chunk.heights.get(row * 17 + col).copied()
}

fn normalise(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    match len > 1.0e-6 {
        true => [v[0] / len, v[1] / len, v[2] / len],
        false => [0.0, 0.0, 1.0],
    }
}

/// **Where a tile's picture goes, and what it is called.**
///
/// The archives key these by an MD5 through `md5translate.trs`, which is a
/// lookup a project cannot join: the index is one flat file for the whole game
/// and an edit that rewrote it would have to carry all 727 KB of it. So an
/// edited picture is written under the name the index *would* have resolved to
/// — the caller reads the real index, finds the tile's MD5, and writes there —
/// and a tile the index has never heard of needs the index edited too.
///
/// [`crate::project::Project`] shadows a virtual path, so writing this path is
/// enough for the editor's own archive overlay to answer with it.
pub fn texture_path(md5_name: &str) -> String {
    format!(r"{}\{md5_name}", vale_assets::tables::minimap::TEXTURE_DIR)
}

/// …and the name the index gives a tile, for building an entry for one it does
/// not carry: `<Map>\map<col>_<row>.blp`.
pub fn index_name(map: &str, tile_x: u32, tile_y: u32) -> String {
    format!(r"{map}\map{tile_x}_{tile_y}.blp")
}

#[cfg(test)]
mod tests {
    use super::*;
    use vale_assets::world::adt::placement_from_world;

    const GRASS: &str = r"Tileset\Elwynn\ElwynnGrassBase.blp";

    fn tile(height: f32) -> Adt {
        let raw = crate::adt::blank_tile(32, 48, GRASS, height, 0).write();
        Adt::parse(&raw).expect("a blank tile parses")
    }

    fn green(_: &str) -> Option<Swatch> {
        Some(Swatch::flat([80, 140, 60]))
    }

    /// A tile with nothing standing on it, which a blank tile is.
    fn nothing(_: &str) -> Option<f32> {
        None
    }

    fn no_shape(_: &str) -> Option<Arc<Shape>> {
        None
    }

    fn no_light(_: Liquid) -> Option<LiquidLight> {
        None
    }

    /// The picture of a tile with the given swatches and canopies and nothing
    /// else, which is what most of these want.
    fn draw(
        adt: &Adt,
        swatch_of: &dyn Fn(&str) -> Option<Swatch>,
        radius_of: &dyn Fn(&str) -> Option<f32>,
    ) -> Vec<u8> {
        render(
            adt,
            &[],
            &Sources {
                swatch_of,
                radius_of,
                shape_of: &no_shape,
                water_of: &no_light,
            },
            &Look::default(),
        )
    }

    /// A blank tile at the given slot with one tree stood at `stood`.
    fn tile_with_a_tree(tile_x: u32, tile_y: u32, stood: [f32; 3]) -> Adt {
        let mut file = crate::adt::blank_tile(tile_x, tile_y, GRASS, 0.0, 0);
        let name_id = file.name_model("world\\tree.m2");
        file.add_doodad(
            crate::adt::Doodad {
                name_id,
                unique_id: 1,
                position: placement_from_world(stood),
                rotation: [0.0; 3],
                scale: 1024,
                flags: 0,
            },
            8.0,
        );
        Adt::parse(&file.write()).expect("parses")
    }

    fn at(px: usize, py: usize) -> usize {
        (py * SIDE + px) * 4
    }

    /// **The picture is the right size and opaque.** The format it ends up in
    /// has no alpha at all, so a renderer leaving any would be writing a
    /// channel that is thrown away — and a half-filled buffer would encode as
    /// black over half the tile.
    #[test]
    fn a_tile_renders_to_an_opaque_square() {
        let picture = draw(&tile(100.0), &green, &nothing);
        assert_eq!(picture.len(), SIDE * SIDE * 4);
        assert!(picture.chunks_exact(4).all(|p| p[3] == 255), "not opaque");
        assert!(
            picture.chunks_exact(4).all(|p| p[..3] != [0, 0, 0]),
            "some pixel was never written"
        );
    }

    /// Flat ground with one texture is one colour, which is the identity this
    /// whole function is a generalisation of. It also pins that the lit term at
    /// a flat normal is not darkening the picture by accident.
    #[test]
    fn flat_ground_under_one_texture_is_one_colour() {
        let picture = draw(&tile(0.0), &green, &nothing);
        let first: [u8; 3] = [picture[0], picture[1], picture[2]];
        for (i, p) in picture.chunks_exact(4).enumerate() {
            assert_eq!(&p[..3], &first, "pixel {i} differs");
        }
        // …and it is recognisably the swatch rather than black or white.
        assert!(first[1] > first[0] && first[1] > first[2], "{first:?} is not green");
    }

    /// One chunk is sixteen pixels, so the picture and the chunk grid line up
    /// exactly. A picture whose chunks straddled pixels would blur every chunk
    /// boundary in the world.
    #[test]
    fn a_chunk_is_a_whole_number_of_pixels() {
        use vale_assets::world::adt::CHUNK_SIZE;
        assert_eq!(PIXELS_PER_CHUNK, 16);
        assert_eq!(PIXELS_PER_CHUNK * CHUNKS_PER_SIDE, SIDE);
        // …and the yards work out to the tile.
        let per_pixel = TILE_SIZE / SIDE as f32;
        assert!((per_pixel * SIDE as f32 - TILE_SIZE).abs() < 0.01);
        assert!((CHUNK_SIZE / per_pixel - PIXELS_PER_CHUNK as f32).abs() < 0.01);
    }

    /// **A texture the caller cannot resolve draws as grey, not as nothing.**
    /// A hole in a minimap reads as water, which is a wrong *shape* rather than
    /// a wrong colour and is much harder to notice as a bug.
    #[test]
    fn an_unresolved_texture_is_grey_rather_than_a_hole() {
        let picture = draw(&tile(0.0), &|_| None, &nothing);
        let first: [u8; 3] = [picture[0], picture[1], picture[2]];
        assert!(first.iter().all(|&c| c > 40), "{first:?} is a hole");
        let spread = first.iter().max().unwrap() - first.iter().min().unwrap();
        assert!(spread < 20, "{first:?} is not grey");
    }

    /// A swatch built from real pixels averages them, which is what makes it a
    /// swatch rather than a crop.
    #[test]
    fn a_swatch_averages_what_it_is_built_from() {
        // Two halves, so the average is knowable.
        let mut rgba = Vec::new();
        for y in 0..8 {
            for _ in 0..8 {
                let c: [u8; 4] = if y < 4 { [0, 0, 0, 255] } else { [200, 200, 200, 255] };
                rgba.extend_from_slice(&c);
            }
        }
        let one = Swatch::from_rgba(&rgba, 8, 8, 1).unwrap();
        assert_eq!(one.side, 1);
        assert_eq!(one.rgb[0], [100, 100, 100]);

        // …and at two a side it keeps the two halves apart.
        let two = Swatch::from_rgba(&rgba, 8, 8, 2).unwrap();
        assert_eq!(two.rgb[0], [0, 0, 0], "top left");
        assert_eq!(two.rgb[3], [200, 200, 200], "bottom right");
    }

    /// A swatch samples wrapped, so the eight repeats across a chunk do not
    /// run off its edge.
    #[test]
    fn a_swatch_wraps() {
        let s = Swatch {
            side: 2,
            rgb: vec![[1, 1, 1], [2, 2, 2], [3, 3, 3], [4, 4, 4]],
        };
        assert_eq!(s.at(0.0, 0.0), s.at(1.0, 1.0));
        assert_eq!(s.at(0.6, 0.0), s.at(7.6, 8.0));
    }

    #[test]
    fn a_swatch_from_nothing_is_nothing() {
        assert!(Swatch::from_rgba(&[], 0, 0, 4).is_none());
        assert!(Swatch::from_rgba(&[0; 4], 4, 4, 2).is_none(), "too few pixels");
    }

    /// The picture encodes to the format the shipped tiles are in, which is the
    /// end this whole module is a means to.
    #[test]
    fn a_rendered_tile_encodes_as_a_minimap_blp() {
        let picture = draw(&tile(0.0), &green, &nothing);
        let blp = vale_assets::world::blp::encode_dxt1(&picture, SIDE as u32, SIDE as u32)
            .expect("encoded");
        assert_eq!(blp.len(), 0x94 + 256 * 4 + 32768, "not the shipped size");
        let back = vale_assets::world::blp::decode(&blp).expect("decoded");
        assert_eq!((back.width, back.height), (SIDE as u32, SIDE as u32));
    }

    /// **A doodad darkens the ground under it and nothing else.**
    ///
    /// The whole of why the first draft looked wrong: a minimap's texture is
    /// its canopies, and drawing none of them is a flat wash. This checks the
    /// disc lands where the placement is rather than merely that something
    /// changed — a footprint in the wrong axis is still a plausible picture.
    #[test]
    fn a_doodad_darkens_the_ground_it_stands_on() {
        // A tree at the centre of chunk 0, which is the picture's top-left
        // sixteen pixels.
        let origin = crate::adt::blank::chunk_origin(32, 48, 0);
        let adt = tile_with_a_tree(32, 48, [origin[0] - 16.0, origin[1] - 16.0, 0.0]);

        let bare = draw(&adt, &green, &nothing);
        let with = draw(&adt, &green, &|_| Some(8.0));

        // Where the tree is: 16 yards in from the corner at 2.08 yards a pixel.
        let under = at(7, 7);
        assert_ne!(
            &with[under..under + 3],
            &bare[under..under + 3],
            "the ground under the tree is unchanged"
        );
        assert!(
            with[under] < bare[under] && with[under + 1] < bare[under + 1],
            "the canopy did not darken the ground"
        );

        // …and the far corner of the tile is untouched, which is what says the
        // disc is a disc and not a wash.
        let away = at(SIDE - 1, SIDE - 1);
        assert_eq!(&with[away..away + 3], &bare[away..away + 3], "the far corner moved");
    }

    /// A model the caller cannot size is skipped rather than guessed at: a
    /// guessed radius that is too large paints a forest over a field.
    #[test]
    fn a_model_with_no_size_is_not_drawn() {
        let bare = tile(0.0);
        let origin = crate::adt::blank::chunk_origin(32, 48, 0);
        let placed = tile_with_a_tree(32, 48, [origin[0] - 16.0, origin[1] - 16.0, 0.0]);
        assert_eq!(draw(&placed, &green, &nothing), draw(&bare, &green, &nothing));
    }

    /// **A neighbour's placement is drawn where it lands on this tile.** The
    /// tree is recorded by tile 33, 48 and stands on 32, 48 — which is what a
    /// model straddling a seam is — and the picture of 32, 48 has it. Drawn
    /// from 32, 48's own file alone, it would not.
    #[test]
    fn a_neighbours_placement_lands_on_this_tile() {
        let origin = crate::adt::blank::chunk_origin(32, 48, 0);
        let stood = [origin[0] - 16.0, origin[1] - 16.0, 0.0];
        let ours = tile(0.0);
        let theirs = tile_with_a_tree(33, 48, stood);
        let canopy = |_: &str| Some(8.0);
        let sources = Sources {
            swatch_of: &green,
            radius_of: &canopy,
            shape_of: &no_shape,
            water_of: &no_light,
        };
        let alone = render(&ours, &[], &sources, &Look::default());
        let with = render(&ours, &[theirs.clone()], &sources, &Look::default());
        let under = at(7, 7);
        assert_ne!(
            &with[under..under + 3],
            &alone[under..under + 3],
            "the neighbour's tree is missing"
        );

        // …and once only, however many tiles record it: the same unique id on
        // both files is one tree, not a darker one.
        let ours_too = tile_with_a_tree(32, 48, stood);
        let twice = render(&ours_too, &[theirs], &sources, &Look::default());
        let once = render(&ours_too, &[], &sources, &Look::default());
        assert_eq!(&twice[under..under + 3], &once[under..under + 3], "drawn twice");
    }

    /// **A building is its roof, seen from above, in its texture's colour** —
    /// and only where it stands above the ground. One red triangle over the
    /// top-left chunk at ten yards up draws red there; the same triangle ten
    /// yards *down* is a cellar and draws nothing.
    #[test]
    fn a_building_draws_its_roof_above_the_ground_only() {
        let mut file = crate::adt::blank_tile(32, 48, GRASS, 0.0, 0);
        let origin = crate::adt::blank::chunk_origin(32, 48, 0);
        let name_id = file.name_building("world\\house.wmo");
        file.add_building(crate::adt::Building {
            name_id,
            unique_id: 7,
            position: placement_from_world([origin[0], origin[1], 0.0]),
            rotation: [0.0; 3],
            bounds_lower: [0.0; 3],
            bounds_upper: [0.0; 3],
            flags: 0,
            doodad_set: 0,
            name_set: 0,
            padding: 0,
        });
        let adt = Adt::parse(&file.write()).expect("parses");

        // The shape is in model space. With no rotation the placement is a
        // half-turn about Z and a translation (`placement_matrix`'s `Rz(pi)`
        // term), so the model's +X/+Y run *against* the world's: a triangle
        // 30 yards along each model axis lands 30 yards in from the tile's
        // corner, over the first chunk.
        let roof = |z: f32| {
            Arc::new(Shape {
                triangles: vec![Triangle {
                    corners: [[0.0, 0.0, z], [30.0, 0.0, z], [0.0, 30.0, z]],
                    colour: [200, 20, 20],
                }],
            })
        };
        let picture = |z: f32| {
            let shape = roof(z);
            let shape_of = move |_: &str| Some(Arc::clone(&shape));
            render(
                &adt,
                &[],
                &Sources {
                    swatch_of: &green,
                    radius_of: &nothing,
                    shape_of: &shape_of,
                    water_of: &no_light,
                },
                &Look::default(),
            )
        };
        let bare = draw(&adt, &green, &nothing);
        let up = picture(10.0);
        let down = picture(-10.0);

        let under = at(2, 2);
        assert!(
            up[under] > 150 && up[under + 1] < 60,
            "the roof is not red: {:?}",
            &up[under..under + 3]
        );
        assert_eq!(&down[under..under + 3], &bare[under..under + 3], "a cellar drew");
        let away = at(SIDE - 1, SIDE - 1);
        assert_eq!(&up[away..away + 3], &bare[away..away + 3], "the far corner moved");
    }

    /// **Water is the light's far colour**, at the light's opacity. A tile
    /// under water with a light that says *red at full opacity* is red; with no
    /// light it is the blue fallback rather than white.
    #[test]
    fn water_is_the_lights_own_colour() {
        // Ground at -20 with the ocean at 0: twenty yards deep everywhere.
        let mut file = crate::adt::blank_tile(32, 48, GRASS, -20.0, 0);
        for index in 0..CHUNKS_PER_SIDE * CHUNKS_PER_SIDE {
            let mut pool = crate::adt::liquid::Pool::flat(Liquid::Ocean, 0.0);
            for row in 0..8 {
                for col in 0..8 {
                    pool.set_wet(row, col, true);
                    // Byte 255: the deep end of the ramp, where the colour
                    // is `far` and nothing of `close`.
                    pool.fill_cell(row, col, 0.0, 255);
                }
            }
            let chunk = file.chunk_mut(index).expect("a blank tile has every chunk");
            crate::adt::liquid::set_pools(chunk, &[pool]);
        }
        let adt = Adt::parse(&file.write()).expect("parses");
        assert!(
            adt.chunks[0].liquids.iter().any(|(k, _)| *k == Liquid::Ocean),
            "no ocean"
        );

        let red = |_: Liquid| {
            Some(LiquidLight {
                close: [0.0, 0.0, 1.0],
                far: [1.0, 0.0, 0.0],
                shallow_alpha: 1.0,
                deep_alpha: 1.0,
            })
        };
        let lit = render(
            &adt,
            &[],
            &Sources {
                swatch_of: &green,
                radius_of: &nothing,
                shape_of: &no_shape,
                water_of: &red,
            },
            &Look::default(),
        );
        assert!(
            lit[0] > 240 && lit[1] < 10 && lit[2] < 10,
            "not the far colour: {:?}",
            &lit[..3]
        );

        let unlit = draw(&adt, &green, &nothing);
        assert!(unlit[2] > unlit[0], "the fallback is not a blue: {:?}", &unlit[..3]);
        assert!(unlit[..3] != [255, 255, 255], "the fallback is white");
    }

    #[test]
    fn the_paths_are_the_games_own() {
        assert_eq!(index_name("Azeroth", 32, 48), r"Azeroth\map32_48.blp");
        assert_eq!(
            texture_path("ea283abc0bf9637c3fad5e840a65b38b.blp"),
            r"textures\Minimap\ea283abc0bf9637c3fad5e840a65b38b.blp"
        );
    }
}
