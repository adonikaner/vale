//! A world map picture: the twelve pieces the 1.12.1 client draws the world,
//! a continent or a zone from, a picture fitted to them or to some of them, a
//! picture drawn from the minimap tiles under a zone's rectangle, the explored
//! areas' art, a continent's zone grid and the highlight sheets.
//!
//! ## The twelve pieces
//!
//! `WorldMapFrame.xml` lays out twelve 256x256 textures, four across and
//! three down, row by row: `Interface\WorldMap\<dir>\<dir>1.blp` at the top
//! left to `<dir>12.blp` at the bottom right. `<dir>` is the `WorldMapArea`
//! row's directory, or `World` for the map of the whole world, which has no
//! row. The pieces cover 1024x768 pixels, and the detail frame shows the
//! top-left [`WIDTH`] by [`HEIGHT`] of them; the border covers the rest. The
//! shipped pieces are DXT1.
//!
//! A picture is therefore made at 1002x668 and cut into the pieces
//! ([`pieces`]). The strip to the right of it and below it is filled with its
//! edge pixels, so a texture filtered at the edge of the shown area reads no
//! stray colour.
//!
//! ## A picture drawn from the minimap
//!
//! A `WorldMapArea` row's rectangle is the part of the world its picture
//! covers: the left edge is world y `left`, the right edge y `right`, the top
//! x `top` and the bottom x `bottom` ([`MapArea`]). The client places the
//! player's marker by the same rectangle, so [`from_minimap`] samples the
//! minimap tiles under it and the marker lands on the ground it stands on.
//! Ground with no minimap tile is drawn black, as the minimap draws it.
//!
//! Every shipped rectangle has the shape of the picture, 1002 by 668, and
//! [`fit_rect`] makes a new one the same shape round what it has to cover.
//!
//! ## Sectors
//!
//! A sector is one piece's part of the shown picture: the last column is 234
//! pixels wide and the last row 156 tall. A set of them is a [`Sectors`] mask,
//! bit `n` for piece `n + 1`. [`paste`] fits a picture to the box the chosen
//! sectors cover and changes only those sectors.
//!
//! ## The explored areas' art
//!
//! A `WorldMapOverlay` row is a picture `width` by `height` drawn at
//! `(offsetX, offsetY)` on a zone's picture when one of its areas is
//! explored. `WorldMapFrame.lua` cuts it into files named
//! `<texture>1`, `<texture>2`, row by row: 256 pixels a side, except the last
//! column and row, whose files are the next power of two of 16 or more over
//! what is left, of which the top-left part is drawn ([`overlay_pieces`]). The
//! shipped pieces are DXT3 with mipmaps, because the art has soft edges.
//!
//! ## The zone grid
//!
//! `Interface\WorldMap\<continent>.zmp` is 128 by 128 little-endian
//! `AreaTable` ids over the whole 64 by 64 tile map, one cell for each half
//! tile: the row from world x and the column from world y, as
//! `vale_assets::tables::worldmap` reads it. It is what names the zone under the
//! pointer on a continent's picture. [`zone_grid`] makes one from the area ids
//! of a map's chunks: each cell takes the id most of the chunks in it carry.
//!
//! ## The highlight sheets
//!
//! On a continent's picture the zone under the pointer is lit by
//! `<dir>\<dir>Highlight.blp`, 128 by 128 DXT1, added to the picture: black
//! leaves it as it is. The sheet's whole width and the top
//! `sheet_fraction` of its height are stretched over the zone's rectangle.
//! The shipped sheets are the zone's shape in a dim green, about
//! (24, 56, 0). [`zone_highlight`] draws one. On the whole world's picture a
//! continent is lit the same way by a 256 by 256 sheet over its tile range
//! ([`continent_highlight`]).

use std::collections::HashMap;
use vale_assets::tables::worldmap::{MapArea, PARCHMENT_HEIGHT, PARCHMENT_WIDTH};
use vale_assets::world::adt::TILE_SIZE;
use vale_assets::world::terrain::position_on_tile;

/// The width and height of the part of the pieces the client shows.
pub const WIDTH: u32 = PARCHMENT_WIDTH as u32;
pub const HEIGHT: u32 = PARCHMENT_HEIGHT as u32;

/// The side of one piece, in pixels.
pub const PIECE: u32 = 256;

/// How many pieces across and down.
pub const ACROSS: u32 = 4;
pub const DOWN: u32 = 3;

/// How many pieces a picture is.
pub const PIECES: u32 = ACROSS * DOWN;

/// The side of one minimap tile's picture, in pixels.
pub const TILE_SIDE: u32 = 256;

/// The directory of the map of the whole world, which has no `WorldMapArea`
/// row.
pub const WORLD: &str = "World";

/// The colour drawn where a piece is missing, and where no minimap tile is.
const BLANK: [u8; 3] = [0, 0, 0];

/// How many samples a side a pixel of a minimap picture averages.
const SAMPLES: u32 = 3;

/// The archive path of piece `n` of the picture in `directory`, `n` from 1.
pub fn piece_path(directory: &str, n: u32) -> String {
    format!("Interface\\WorldMap\\{directory}\\{directory}{n}.blp")
}

/// One piece's part of the shown picture, 0-based: left, top, right and
/// bottom, in pixels.
pub fn sector_rect(index: u32) -> [u32; 4] {
    let (left, top) = ((index % ACROSS) * PIECE, (index / ACROSS) * PIECE);
    [left, top, (left + PIECE).min(WIDTH), (top + PIECE).min(HEIGHT)]
}

/// A set of sectors: bit `n` for piece `n + 1`.
pub type Sectors = u16;

/// All twelve.
pub const EVERY: Sectors = (1 << PIECES) - 1;

/// The box the sectors in `sectors` cover together, or `None` for none.
pub fn sectors_box(sectors: Sectors) -> Option<[u32; 4]> {
    (0..PIECES).filter(|n| sectors & (1 << n) != 0).map(sector_rect).reduce(|a, b| {
        [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]
    })
}

/// How a picture of another shape than 1002x668 is fitted to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Fit {
    /// Scaled to 1002x668, whatever its shape.
    #[default]
    Stretch,
    /// Scaled to cover 1002x668, and the middle of it kept.
    Crop,
}

/// The picture `rgba`, `width` by `height`, scaled to [`WIDTH`] by
/// [`HEIGHT`] as `fit` says, as opaque RGBA. A pixel's alpha is applied over
/// black, as a DXT1 piece with no alpha shows it.
pub fn fit(rgba: &[u8], width: u32, height: u32, fit: Fit) -> Vec<u8> {
    fit_to(rgba, width, height, fit, WIDTH, HEIGHT, false)
}

/// The picture `rgba`, `width` by `height`, scaled to `out_w` by `out_h` as
/// `fit` says. With `keep_alpha` the alpha is kept; without it each pixel is
/// opaque and its alpha is applied over black.
pub fn fit_to(rgba: &[u8], width: u32, height: u32, fit: Fit, out_w: u32, out_h: u32, keep_alpha: bool) -> Vec<u8> {
    let (w, h) = (width.max(1) as f32, height.max(1) as f32);
    let window = match fit {
        Fit::Stretch => [0.0, 0.0, w, h],
        Fit::Crop => {
            let scale = (out_w.max(1) as f32 / w).max(out_h.max(1) as f32 / h);
            let (kept_w, kept_h) = (out_w as f32 / scale, out_h as f32 / scale);
            let (x0, y0) = ((w - kept_w) / 2.0, (h - kept_h) / 2.0);
            [x0, y0, x0 + kept_w, y0 + kept_h]
        }
    };
    resample(rgba, width, height, window, out_w, out_h, keep_alpha)
}

/// Whether a picture `width` by `height` has the shape of 1002x668, to within
/// a pixel at that size, so [`Fit`] makes no difference to it.
pub fn has_the_shape(width: u32, height: u32) -> bool {
    same_shape(width, height, WIDTH, HEIGHT)
}

/// Whether a picture `width` by `height` has the shape of `out_w` by `out_h`,
/// to within a pixel at that size.
pub fn same_shape(width: u32, height: u32, out_w: u32, out_h: u32) -> bool {
    let fitted = height as f32 * out_w as f32 / width.max(1) as f32;
    (fitted - out_h as f32).abs() < 1.0
}

/// `picture`, `width` by `height`, fitted as `fit` says to the box
/// [`sectors_box`] gives for `sectors`, and written into `onto`, a [`WIDTH`]
/// by [`HEIGHT`] picture, inside those sectors only.
pub fn paste(onto: &mut [u8], picture: &[u8], width: u32, height: u32, fit: Fit, sectors: Sectors) {
    let Some([left, top, right, bottom]) = sectors_box(sectors) else { return };
    let (w, h) = (right - left, bottom - top);
    let fitted = fit_to(picture, width, height, fit, w, h, false);
    for index in (0..PIECES).filter(|n| sectors & (1 << n) != 0) {
        let [l, t, r, b] = sector_rect(index);
        for y in t..b {
            let from = (((y - top) * w + (l - left)) * 4) as usize;
            let to = ((y * WIDTH + l) * 4) as usize;
            let run = ((r - l) * 4) as usize;
            onto[to..to + run].copy_from_slice(&fitted[from..from + run]);
        }
    }
}

/// The sectors in `sectors` of `from` copied into `onto`, both [`WIDTH`] by
/// [`HEIGHT`].
pub fn copy_sectors(onto: &mut [u8], from: &[u8], sectors: Sectors) {
    for index in (0..PIECES).filter(|n| sectors & (1 << n) != 0) {
        let [l, t, r, b] = sector_rect(index);
        for y in t..b {
            let at = ((y * WIDTH + l) * 4) as usize;
            let run = ((r - l) * 4) as usize;
            onto[at..at + run].copy_from_slice(&from[at..at + run]);
        }
    }
}

/// The part `window` (left, top, right, bottom, in source pixels) of a
/// picture, scaled to `out_w` by `out_h`. Each pixel averages a grid of
/// bilinear samples, as many a side as source pixels fall across it, so a
/// picture made smaller is averaged rather than skipped through. The colours
/// are averaged weighted by their alpha; with `keep_alpha` the alpha is the
/// average alpha, and without it the pixel is opaque over black.
fn resample(
    rgba: &[u8],
    width: u32,
    height: u32,
    window: [f32; 4],
    out_w: u32,
    out_h: u32,
    keep_alpha: bool,
) -> Vec<u8> {
    let mut out = vec![0u8; (out_w * out_h * 4) as usize];
    if width == 0 || height == 0 || rgba.len() < (width * height * 4) as usize {
        for pixel in out.chunks_exact_mut(4) {
            pixel[3] = if keep_alpha { 0 } else { 255 };
        }
        return out;
    }
    let step = [(window[2] - window[0]) / out_w as f32, (window[3] - window[1]) / out_h as f32];
    let grid = |step: f32| (step.ceil() as u32).clamp(1, 8);
    let (gx, gy) = (grid(step[0]), grid(step[1]));
    let at = |x: u32, y: u32| ((y.min(height - 1) * width + x.min(width - 1)) * 4) as usize;
    let bilinear = |x: f32, y: f32| {
        let x = (x - 0.5).clamp(0.0, (width - 1) as f32);
        let y = (y - 0.5).clamp(0.0, (height - 1) as f32);
        let (x0, y0) = (x.floor() as u32, y.floor() as u32);
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let mut colour = [0f32; 4];
        for (dx, dy, weight) in [
            (0, 0, (1.0 - fx) * (1.0 - fy)),
            (1, 0, fx * (1.0 - fy)),
            (0, 1, (1.0 - fx) * fy),
            (1, 1, fx * fy),
        ] {
            let i = at(x0 + dx, y0 + dy);
            for c in 0..4 {
                colour[c] += rgba[i + c] as f32 * weight;
            }
        }
        colour
    };
    for v in 0..out_h {
        for u in 0..out_w {
            let mut sum = [0f32; 4];
            for sy in 0..gy {
                for sx in 0..gx {
                    let x = window[0] + (u as f32 + (sx as f32 + 0.5) / gx as f32) * step[0];
                    let y = window[1] + (v as f32 + (sy as f32 + 0.5) / gy as f32) * step[1];
                    let colour = bilinear(x, y);
                    // Premultiplied, so a transparent pixel adds black.
                    let alpha = colour[3] / 255.0;
                    for c in 0..3 {
                        sum[c] += colour[c] * alpha;
                    }
                    sum[3] += alpha;
                }
            }
            let count = (gx * gy) as f32;
            let i = ((v * out_w + u) * 4) as usize;
            let byte = |value: f32| value.round().clamp(0.0, 255.0) as u8;
            match keep_alpha {
                true => {
                    let alpha = sum[3] / count;
                    for c in 0..3 {
                        out[i + c] = match sum[3] > 0.0 {
                            true => byte(sum[c] / sum[3]),
                            false => 0,
                        };
                    }
                    out[i + 3] = byte(alpha * 255.0);
                }
                false => {
                    for c in 0..3 {
                        out[i + c] = byte(sum[c] / count);
                    }
                    out[i + 3] = 255;
                }
            }
        }
    }
    out
}

/// A [`WIDTH`] by [`HEIGHT`] picture cut into the twelve [`PIECE`]-square
/// pieces, in file order, each as RGBA. Pixels past the picture's right and
/// bottom edges repeat its edge.
pub fn pieces(picture: &[u8]) -> Vec<Vec<u8>> {
    (0..PIECES)
        .map(|index| {
            let (left, top) = ((index % ACROSS) * PIECE, (index / ACROSS) * PIECE);
            let mut piece = vec![0u8; (PIECE * PIECE * 4) as usize];
            for y in 0..PIECE {
                for x in 0..PIECE {
                    let sx = (left + x).min(WIDTH - 1);
                    let sy = (top + y).min(HEIGHT - 1);
                    let from = ((sy * WIDTH + sx) * 4) as usize;
                    let to = ((y * PIECE + x) * 4) as usize;
                    piece[to..to + 4].copy_from_slice(&picture[from..from + 4]);
                }
            }
            piece
        })
        .collect()
}

/// The picture twelve pieces show, [`WIDTH`] by [`HEIGHT`] RGBA, and how many
/// of the pieces were found. `piece` answers piece `n`'s RGBA pixels, `n`
/// from 1, or `None` where there is none; a piece of another size than
/// [`PIECE`] square counts as missing. A missing piece is drawn black.
pub fn join(mut piece: impl FnMut(u32) -> Option<(u32, u32, Vec<u8>)>) -> (Vec<u8>, u32) {
    let mut out = vec![0u8; (WIDTH * HEIGHT * 4) as usize];
    let mut found = 0;
    for index in 0..PIECES {
        let (left, top) = ((index % ACROSS) * PIECE, (index / ACROSS) * PIECE);
        let pixels = piece(index + 1)
            .filter(|(w, h, rgba)| *w == PIECE && *h == PIECE && rgba.len() == (PIECE * PIECE * 4) as usize)
            .map(|(_, _, rgba)| rgba);
        found += u32::from(pixels.is_some());
        for y in top..(top + PIECE).min(HEIGHT) {
            for x in left..(left + PIECE).min(WIDTH) {
                let to = ((y * WIDTH + x) * 4) as usize;
                let colour = match &pixels {
                    Some(rgba) => {
                        let from = (((y - top) * PIECE + (x - left)) * 4) as usize;
                        [rgba[from], rgba[from + 1], rgba[from + 2]]
                    }
                    None => BLANK,
                };
                out[to..to + 3].copy_from_slice(&colour);
                out[to + 3] = 255;
            }
        }
    }
    (out, found)
}

/// A picture drawn from the minimap, and how many minimap tiles were asked
/// for and drawn.
#[derive(Debug, Clone, PartialEq)]
pub struct Drawn {
    /// [`WIDTH`] by [`HEIGHT`] RGBA.
    pub picture: Vec<u8>,
    /// Tiles under the rectangle that had a minimap picture.
    pub drawn: usize,
    /// Tiles under the rectangle that had none: ground that is not there, or
    /// a tile whose minimap picture has not been drawn.
    pub missing: usize,
}

/// The rectangle of `area` drawn from the minimap tiles under it, north at
/// the top and west on the left. `tile` answers one minimap tile's RGBA
/// pixels, [`TILE_SIDE`] square, or `None` where there is none; it is asked
/// once per tile. `row_done` is told each finished row, for a progress bar.
///
/// Each tile is shrunk, when it is read, to the size it is drawn at with
/// [`SAMPLES`] samples a pixel, so a continent of a thousand tiles keeps a few
/// megabytes of them rather than a quarter of a gigabyte.
pub fn from_minimap(
    area: &MapArea,
    mut tile: impl FnMut(u32, u32) -> Option<Vec<u8>>,
    mut row_done: impl FnMut(u32),
) -> Drawn {
    let span = [area.left - area.right, area.top - area.bottom];
    // Pixels of the picture across one tile, on the axis that needs more.
    let per_tile = (WIDTH as f32 * TILE_SIZE / span[0].abs().max(1.0))
        .max(HEIGHT as f32 * TILE_SIZE / span[1].abs().max(1.0));
    let side = ((per_tile * SAMPLES as f32).ceil() as u32).clamp(8, TILE_SIDE);
    let mut tiles: HashMap<(u32, u32), Option<Vec<u8>>> = HashMap::new();
    let mut out = vec![0u8; (WIDTH * HEIGHT * 4) as usize];
    for v in 0..HEIGHT {
        for u in 0..WIDTH {
            let mut sum = [0u32; 3];
            for sv in 0..SAMPLES {
                for su in 0..SAMPLES {
                    let fu = (u as f32 + (su as f32 + 0.5) / SAMPLES as f32) / WIDTH as f32;
                    let fv = (v as f32 + (sv as f32 + 0.5) / SAMPLES as f32) / HEIGHT as f32;
                    // Across the picture is east (falling y), down it is
                    // south (falling x).
                    let y = area.left - fu * span[0];
                    let x = area.top - fv * span[1];
                    let colour = match position_on_tile(x, y) {
                        Some((at, [across, down])) => {
                            let pixels = tiles
                                .entry(at)
                                .or_insert_with(|| tile(at.0, at.1).filter(|p| p.len() == (TILE_SIDE * TILE_SIDE * 4) as usize).map(|p| shrink(&p, side)));
                            match pixels {
                                Some(pixels) => {
                                    let tu = ((across * side as f32) as u32).min(side - 1);
                                    let tv = ((down * side as f32) as u32).min(side - 1);
                                    let i = ((tv * side + tu) * 4) as usize;
                                    [pixels[i], pixels[i + 1], pixels[i + 2]]
                                }
                                None => BLANK,
                            }
                        }
                        None => BLANK,
                    };
                    for (total, value) in sum.iter_mut().zip(colour) {
                        *total += u32::from(value);
                    }
                }
            }
            let i = ((v * WIDTH + u) * 4) as usize;
            for c in 0..3 {
                out[i + c] = (sum[c] / (SAMPLES * SAMPLES)) as u8;
            }
            out[i + 3] = 255;
        }
        row_done(v + 1);
    }
    let drawn = tiles.values().filter(|pixels| pixels.is_some()).count();
    Drawn { picture: out, drawn, missing: tiles.len() - drawn }
}

/// A [`TILE_SIDE`]-square picture averaged down to `side` square.
fn shrink(rgba: &[u8], side: u32) -> Vec<u8> {
    if side >= TILE_SIDE {
        return rgba.to_vec();
    }
    resample(rgba, TILE_SIDE, TILE_SIDE, [0.0, 0.0, TILE_SIDE as f32, TILE_SIDE as f32], side, side, false)
}

/// The world rectangle of the picture's shape that covers `bounds` (least x,
/// least y, most x, most y, in world yards) with a margin of a twentieth on
/// each side, as a `WorldMapArea` row holds it: left, right, top, bottom,
/// where left and top are the larger y and x.
pub fn fit_rect(bounds: [f32; 4]) -> [f32; 4] {
    const MARGIN: f32 = 0.05;
    const LEAST: f32 = 100.0;
    let centre = [(bounds[0] + bounds[2]) / 2.0, (bounds[1] + bounds[3]) / 2.0];
    // Across the picture is y; down it is x.
    let across = (bounds[3] - bounds[1]).max(LEAST) * (1.0 + 2.0 * MARGIN);
    let down = (bounds[2] - bounds[0]).max(LEAST) * (1.0 + 2.0 * MARGIN);
    let shape = WIDTH as f32 / HEIGHT as f32;
    let (across, down) = match across / down > shape {
        true => (across, across / shape),
        false => (down * shape, down),
    };
    [centre[1] + across / 2.0, centre[1] - across / 2.0, centre[0] + down / 2.0, centre[0] - down / 2.0]
}

/// The archive path of piece `n` of an overlay, `n` from 1.
pub fn overlay_piece_path(directory: &str, texture: &str, n: u32) -> String {
    format!("Interface\\WorldMap\\{directory}\\{texture}{n}.blp")
}

/// How many files across and down an overlay `width` by `height` is.
pub fn overlay_grid(width: u32, height: u32) -> (u32, u32) {
    (width.div_ceil(PIECE).max(1), height.div_ceil(PIECE).max(1))
}

/// The pixels the last file of a row or column shows, and the file's side:
/// 256 for every file but the last, and for the last what is left and the next
/// power of two of 16 or more over it.
fn overlay_span(size: u32, index: u32, count: u32) -> (u32, u32) {
    if index + 1 < count {
        return (PIECE, PIECE);
    }
    let shown = match size % PIECE {
        0 => PIECE,
        left => left,
    };
    (shown, shown.next_power_of_two().max(16))
}

/// An overlay's art, `width` by `height` RGBA, cut into its files in file
/// order, each as its side across, its side down and its RGBA. The part of a
/// file past what it shows repeats the art's edge.
pub fn overlay_pieces(rgba: &[u8], width: u32, height: u32) -> Vec<(u32, u32, Vec<u8>)> {
    let (wide, tall) = overlay_grid(width, height);
    let mut out = Vec::new();
    for j in 0..tall {
        let (_, file_h) = overlay_span(height, j, tall);
        for k in 0..wide {
            let (_, file_w) = overlay_span(width, k, wide);
            let mut piece = vec![0u8; (file_w * file_h * 4) as usize];
            for y in 0..file_h {
                for x in 0..file_w {
                    let sx = (k * PIECE + x).min(width.max(1) - 1);
                    let sy = (j * PIECE + y).min(height.max(1) - 1);
                    let from = ((sy * width + sx) * 4) as usize;
                    let to = ((y * file_w + x) * 4) as usize;
                    if from + 4 <= rgba.len() {
                        piece[to..to + 4].copy_from_slice(&rgba[from..from + 4]);
                    }
                }
            }
            out.push((file_w, file_h, piece));
        }
    }
    out
}

/// An overlay's art, `width` by `height` RGBA, joined from its files.
/// `piece` answers file `n`'s side across, side down and RGBA, `n` from 1, or
/// `None` where there is none, which leaves that part clear.
pub fn join_overlay(width: u32, height: u32, mut piece: impl FnMut(u32) -> Option<(u32, u32, Vec<u8>)>) -> Vec<u8> {
    let mut out = vec![0u8; (width * height * 4) as usize];
    let (wide, tall) = overlay_grid(width, height);
    for j in 0..tall {
        let (shown_h, _) = overlay_span(height, j, tall);
        for k in 0..wide {
            let (shown_w, _) = overlay_span(width, k, wide);
            let Some((file_w, file_h, rgba)) = piece(j * wide + k + 1) else { continue };
            if rgba.len() < (file_w * file_h * 4) as usize {
                continue;
            }
            for y in 0..shown_h.min(file_h) {
                for x in 0..shown_w.min(file_w) {
                    let (ox, oy) = (k * PIECE + x, j * PIECE + y);
                    if ox >= width || oy >= height {
                        continue;
                    }
                    let from = ((y * file_w + x) * 4) as usize;
                    let to = ((oy * width + ox) * 4) as usize;
                    out[to..to + 4].copy_from_slice(&rgba[from..from + 4]);
                }
            }
        }
    }
    out
}

/// `art`, `width` by `height` RGBA, drawn over `onto`, a [`WIDTH`] by
/// [`HEIGHT`] picture, with its top-left corner at `(left, top)`, by its
/// alpha.
pub fn blend(onto: &mut [u8], art: &[u8], width: u32, height: u32, left: u32, top: u32) {
    for y in 0..height {
        let oy = top + y;
        if oy >= HEIGHT {
            break;
        }
        for x in 0..width {
            let ox = left + x;
            if ox >= WIDTH {
                break;
            }
            let from = ((y * width + x) * 4) as usize;
            let to = ((oy * WIDTH + ox) * 4) as usize;
            let alpha = u32::from(art[from + 3]);
            for c in 0..3 {
                let mixed = (u32::from(art[from + c]) * alpha + u32::from(onto[to + c]) * (255 - alpha)) / 255;
                onto[to + c] = mixed as u8;
            }
        }
    }
}

/// The label rectangle a new overlay starts with: the middle half of its
/// art, as top, left, bottom and right.
pub fn default_hit(width: u32, height: u32, left: u32, top: u32) -> [u32; 4] {
    [top + height / 4, left + width / 4, top + height * 3 / 4, left + width * 3 / 4]
}

/// The side of the zone grid, in cells.
pub const GRID_SIDE: usize = 128;

/// The whole map's side, in yards: 64 tiles.
const MAP_YARDS: f32 = TILE_SIZE * 64.0;

/// The grid cell a world position falls in, as an index, or `None` off the
/// map.
pub fn grid_cell(x: f32, y: f32) -> Option<usize> {
    let axis = |value: f32| {
        let normalised = 0.5 - value / MAP_YARDS;
        (0.0..1.0).contains(&normalised).then(|| ((normalised * GRID_SIDE as f32) as usize).min(GRID_SIDE - 1))
    };
    Some(axis(x)? * GRID_SIDE + axis(y)?)
}

/// The zone grid of a map from its chunks, each given as the world x and y of
/// its middle and its area id. Each cell holds the id most of the chunks in
/// it carry, the lower id on a tie, and 0 where there is no chunk.
pub fn zone_grid(chunks: impl IntoIterator<Item = (f32, f32, u32)>) -> Vec<u32> {
    let mut votes: HashMap<usize, HashMap<u32, u32>> = HashMap::new();
    for (x, y, area) in chunks {
        if let Some(cell) = grid_cell(x, y) {
            *votes.entry(cell).or_default().entry(area).or_default() += 1;
        }
    }
    let mut grid = vec![0u32; GRID_SIDE * GRID_SIDE];
    for (cell, counts) in votes {
        let best = counts.iter().max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0))).map_or(0, |(id, _)| *id);
        grid[cell] = best;
    }
    grid
}

/// A zone grid as the `.zmp` file holds it.
pub fn zmp_bytes(grid: &[u32]) -> Vec<u8> {
    grid.iter().flat_map(|id| id.to_le_bytes()).collect()
}

/// A `.zmp` file's grid, or `None` for a file too short to be one.
pub fn read_zmp(bytes: &[u8]) -> Option<Vec<u32>> {
    let cells = GRID_SIDE * GRID_SIDE;
    (bytes.len() >= cells * 4).then(|| {
        bytes.chunks_exact(4).take(cells).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
    })
}

/// The side of a zone's highlight sheet and of a continent's.
pub const ZONE_SHEET: u32 = 128;
pub const CONTINENT_SHEET: u32 = 256;

/// The colour a highlight sheet lights a zone or a continent with.
const GLOW: [f32; 3] = [24.0, 56.0, 0.0];

/// A sheet `side` square: the top-left `fraction` (across, down) of it is the
/// unit square, lit where `inside(u, v)` is, with four samples a side so the
/// edge is soft; the rest is black. Opaque RGBA.
fn sheet(side: u32, fraction: (f32, f32), inside: impl Fn(f32, f32) -> bool) -> Vec<u8> {
    const SUB: u32 = 4;
    let mut out = vec![0u8; (side * side * 4) as usize];
    for t in 0..side {
        for s in 0..side {
            let mut lit = 0;
            for sy in 0..SUB {
                for sx in 0..SUB {
                    let u = (s as f32 + (sx as f32 + 0.5) / SUB as f32) / side as f32 / fraction.0.max(1e-6);
                    let v = (t as f32 + (sy as f32 + 0.5) / SUB as f32) / side as f32 / fraction.1.max(1e-6);
                    if u < 1.0 && v < 1.0 && inside(u, v) {
                        lit += 1;
                    }
                }
            }
            let share = lit as f32 / (SUB * SUB) as f32;
            let i = ((t * side + s) * 4) as usize;
            for c in 0..3 {
                out[i + c] = (GLOW[c] * share).round() as u8;
            }
            out[i + 3] = 255;
        }
    }
    out
}

/// A zone's highlight sheet, [`ZONE_SHEET`] square: the zone's rectangle over
/// the whole width and the top `fraction_down` of the height, lit where
/// `inside(x, y)` says a world position is in the zone.
pub fn zone_highlight(area: &MapArea, fraction_down: f32, inside: impl Fn(f32, f32) -> bool) -> Vec<u8> {
    let span = [area.left - area.right, area.top - area.bottom];
    sheet(ZONE_SHEET, (1.0, fraction_down), |u, v| inside(area.top - v * span[1], area.left - u * span[0]))
}

/// A continent's highlight sheet, [`CONTINENT_SHEET`] square, over its tile
/// range `[first column, last column, first row, last row]`: the longer of
/// the two spans takes the whole sheet and the other its share, as the client
/// fits it, and a tile is lit when `has(column, row)`.
pub fn continent_highlight(range: [u32; 4], has: impl Fn(u32, u32) -> bool) -> Vec<u8> {
    let across = (range[1].saturating_sub(range[0]) + 1) as f32;
    let down = (range[3].saturating_sub(range[2]) + 1) as f32;
    let fraction = match across > down {
        true => (1.0, down / across),
        false => (across / down, 1.0),
    };
    sheet(CONTINENT_SHEET, fraction, |u, v| {
        has(range[0] + (u * across) as u32, range[2] + (v * down) as u32)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use vale_assets::world::adt::MAP_ORIGIN;

    fn at(picture: &[u8], width: u32, x: u32, y: u32) -> [u8; 3] {
        let i = ((y * width + x) * 4) as usize;
        [picture[i], picture[i + 1], picture[i + 2]]
    }

    /// A picture the size of the pieces' shown area, red in its left half and
    /// blue in its right.
    fn halves(width: u32, height: u32) -> Vec<u8> {
        let mut rgba = vec![255u8; (width * height * 4) as usize];
        for y in 0..height {
            for x in 0..width {
                let i = ((y * width + x) * 4) as usize;
                let colour = if x < width / 2 { [255, 0, 0] } else { [0, 0, 255] };
                rgba[i..i + 3].copy_from_slice(&colour);
            }
        }
        rgba
    }

    /// The pieces are named after their directory and numbered from 1.
    #[test]
    fn a_piece_is_named_after_its_directory() {
        assert_eq!(piece_path("Elwynn", 1), "Interface\\WorldMap\\Elwynn\\Elwynn1.blp");
        assert_eq!(piece_path(WORLD, 12), "Interface\\WorldMap\\World\\World12.blp");
        assert_eq!((WIDTH, HEIGHT), (1002, 668));
    }

    /// Twelve pieces in rows of four; the strip past the shown area repeats
    /// its edge; and joined again, the pieces are the picture.
    #[test]
    fn a_picture_is_cut_into_twelve_pieces_and_joined_again() {
        let picture = halves(WIDTH, HEIGHT);
        let cut = pieces(&picture);
        assert_eq!(cut.len(), 12);
        assert!(cut.iter().all(|piece| piece.len() == (PIECE * PIECE * 4) as usize));
        assert_eq!(at(&cut[0], PIECE, 0, 0), [255, 0, 0], "piece 1 is the top left");
        assert_eq!(at(&cut[3], PIECE, 0, 0), [0, 0, 255], "piece 4 is the top right");
        // Piece 4 starts at x 768; the picture ends at x 1001, and x 1002 on
        // repeats it.
        assert_eq!(at(&cut[3], PIECE, 255, 0), [0, 0, 255]);
        // Piece 12 starts at y 512; the picture ends at y 667.
        assert_eq!(at(&cut[11], PIECE, 255, 255), [0, 0, 255]);
        let (joined, found) = join(|n| Some((PIECE, PIECE, cut[(n - 1) as usize].clone())));
        assert_eq!(found, 12);
        assert_eq!(joined, picture);
    }

    /// A missing piece, or one of the wrong size, is drawn black and not
    /// counted.
    #[test]
    fn a_missing_piece_is_drawn_black() {
        let white = vec![255u8; (PIECE * PIECE * 4) as usize];
        let (joined, found) = join(|n| match n {
            1 => None,
            2 => Some((128, 128, vec![255u8; 128 * 128 * 4])),
            _ => Some((PIECE, PIECE, white.clone())),
        });
        assert_eq!(found, 10);
        assert_eq!(at(&joined, WIDTH, 10, 10), [0, 0, 0]);
        assert_eq!(at(&joined, WIDTH, 300, 10), [0, 0, 0]);
        assert_eq!(at(&joined, WIDTH, 600, 10), [255, 255, 255]);
    }

    /// Stretch keeps the whole picture; Crop keeps its middle. A square
    /// picture, red left and blue right, keeps both colours stretched and
    /// cropped, and cropped its middle is where the colours meet.
    #[test]
    fn a_picture_of_another_shape_is_stretched_or_cropped() {
        let square = halves(400, 400);
        let stretched = fit(&square, 400, 400, Fit::Stretch);
        assert_eq!(stretched.len(), (WIDTH * HEIGHT * 4) as usize);
        assert_eq!(at(&stretched, WIDTH, 5, 5), [255, 0, 0]);
        assert_eq!(at(&stretched, WIDTH, WIDTH - 5, HEIGHT - 5), [0, 0, 255]);
        // Cropped, the square is scaled to 1002 across and its top and bottom
        // cut, so the left and right edges are still the square's.
        let cropped = fit(&square, 400, 400, Fit::Crop);
        assert_eq!(at(&cropped, WIDTH, 5, HEIGHT / 2), [255, 0, 0]);
        assert_eq!(at(&cropped, WIDTH, WIDTH - 5, HEIGHT / 2), [0, 0, 255]);
        // A tall picture cropped loses its left and right instead: a picture
        // whose middle third is green is all green.
        let (w, h) = (300u32, 900u32);
        let mut tall = vec![255u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                let colour = if (h / 3..2 * h / 3).contains(&y) { [0, 255, 0] } else { [255, 0, 0] };
                tall[i..i + 3].copy_from_slice(&colour);
            }
        }
        let cropped = fit(&tall, w, h, Fit::Crop);
        assert_eq!(at(&cropped, WIDTH, 5, 5), [0, 255, 0]);
        assert_eq!(at(&cropped, WIDTH, WIDTH - 5, HEIGHT - 5), [0, 255, 0]);
        assert!(has_the_shape(2004, 1336) && has_the_shape(WIDTH, HEIGHT));
        assert!(!has_the_shape(400, 400));
    }

    /// A transparent pixel is drawn black, as a piece with no alpha shows it.
    #[test]
    fn a_transparent_picture_is_drawn_over_black() {
        let clear = [255, 255, 255, 0].repeat((WIDTH * HEIGHT) as usize);
        let fitted = fit(&clear, WIDTH, HEIGHT, Fit::Stretch);
        assert_eq!(at(&fitted, WIDTH, 100, 100), [0, 0, 0]);
        assert_eq!(fitted[3], 255);
    }

    /// The rectangle of one tile, whose minimap picture is dark in its
    /// north-west quarter, is drawn dark at the top left: north at the top,
    /// west on the left. The tile is asked for once.
    #[test]
    fn a_minimap_picture_has_north_at_the_top_and_west_on_the_left() {
        let north = MAP_ORIGIN - 30.0 * TILE_SIZE;
        let west = MAP_ORIGIN - 30.0 * TILE_SIZE;
        let area = MapArea {
            id: 1,
            map: 0,
            area: 12,
            directory: "Test".to_string(),
            left: west,
            right: west - TILE_SIZE,
            top: north,
            bottom: north - TILE_SIZE,
        };
        let mut pixels = vec![255u8; (TILE_SIDE * TILE_SIDE * 4) as usize];
        for v in 0..TILE_SIDE / 2 {
            for u in 0..TILE_SIDE / 2 {
                let i = ((v * TILE_SIDE + u) * 4) as usize;
                pixels[i..i + 3].copy_from_slice(&[0, 0, 0]);
            }
        }
        let mut asked = Vec::new();
        let mut rows = 0;
        let drawn = from_minimap(
            &area,
            |col, row| {
                asked.push((col, row));
                Some(pixels.clone())
            },
            |row| rows = row,
        );
        let at = |u: u32, v: u32| drawn.picture[((v * WIDTH + u) * 4) as usize];
        assert_eq!(at(10, 10), 0, "north-west is dark");
        assert_eq!(at(WIDTH - 10, 10), 255, "north-east is light");
        assert_eq!(at(10, HEIGHT - 10), 255, "south-west is light");
        assert_eq!(asked, vec![(30, 30)], "one tile, asked once");
        assert_eq!((drawn.drawn, drawn.missing), (1, 0));
        assert_eq!(rows, HEIGHT);
        // A position the client maps to the middle of the picture is drawn
        // from the middle of the rectangle.
        let (u, v) = area.position(north - TILE_SIZE / 2.0, west - TILE_SIZE / 2.0).unwrap();
        assert!((u - 0.5).abs() < 1e-4 && (v - 0.5).abs() < 1e-4);
    }

    /// The sectors' boxes, and a picture pasted into two of them changes
    /// those two and no other.
    #[test]
    fn a_picture_is_pasted_into_the_chosen_sectors_only() {
        assert_eq!(sector_rect(0), [0, 0, 256, 256]);
        assert_eq!(sector_rect(3), [768, 0, WIDTH, 256]);
        assert_eq!(sector_rect(11), [768, 512, WIDTH, HEIGHT]);
        assert_eq!(sectors_box(EVERY), Some([0, 0, WIDTH, HEIGHT]));
        assert_eq!(sectors_box(0), None);
        // Pieces 1 and 6: the top-left one and the one diagonally below it.
        let chosen: Sectors = 1 | (1 << 5);
        assert_eq!(sectors_box(chosen), Some([0, 0, 512, 512]));
        let mut picture = vec![0u8; (WIDTH * HEIGHT * 4) as usize];
        let white = vec![255u8; 16 * 16 * 4];
        paste(&mut picture, &white, 16, 16, Fit::Stretch, chosen);
        assert_eq!(at(&picture, WIDTH, 10, 10), [255, 255, 255]);
        assert_eq!(at(&picture, WIDTH, 300, 300), [255, 255, 255]);
        assert_eq!(at(&picture, WIDTH, 300, 10), [0, 0, 0], "piece 2 is not chosen");
        let mut other = vec![0u8; (WIDTH * HEIGHT * 4) as usize];
        copy_sectors(&mut other, &picture, 1 << 5);
        assert_eq!(at(&other, WIDTH, 10, 10), [0, 0, 0]);
        assert_eq!(at(&other, WIDTH, 300, 300), [255, 255, 255]);
    }

    /// An overlay of 300 by 240 is two files across and one down; the second
    /// file is 64 wide, the next power of two over the 44 pixels it shows; and
    /// the files join back into the art, which keeps its alpha.
    #[test]
    fn an_overlay_is_cut_as_the_frame_lays_it_out_and_joined_again() {
        let (w, h) = (300u32, 240u32);
        let mut art = vec![0u8; (w * h * 4) as usize];
        for (i, pixel) in art.chunks_exact_mut(4).enumerate() {
            pixel.copy_from_slice(&[(i % 251) as u8, 7, 9, (i % 256) as u8]);
        }
        let cut = overlay_pieces(&art, w, h);
        assert_eq!(cut.iter().map(|(a, b, _)| (*a, *b)).collect::<Vec<_>>(), [(256, 256), (64, 256)]);
        assert_eq!(overlay_grid(512, 257), (2, 2));
        assert_eq!(overlay_span(257, 1, 2), (1, 16), "the least file is 16");
        let joined = join_overlay(w, h, |n| cut.get(n as usize - 1).cloned());
        assert_eq!(joined, art);
        assert_eq!(
            overlay_piece_path("Elwynn", "GOLDSHIRE", 2),
            "Interface\\WorldMap\\Elwynn\\GOLDSHIRE2.blp"
        );
        // Drawn over a black picture, a fully opaque pixel replaces it and a
        // clear one leaves it.
        let mut picture = vec![0u8; (WIDTH * HEIGHT * 4) as usize];
        let two = [255, 255, 255, 255, 255, 255, 255, 0];
        blend(&mut picture, &two, 2, 1, 10, 20);
        assert_eq!(at(&picture, WIDTH, 10, 20), [255, 255, 255]);
        assert_eq!(at(&picture, WIDTH, 11, 20), [0, 0, 0]);
        assert_eq!(default_hit(200, 100, 10, 20), [45, 60, 95, 160]);
        // Fitting with alpha kept keeps a clear picture clear.
        let clear = [9, 9, 9, 0].repeat(16);
        assert_eq!(fit_to(&clear, 4, 4, Fit::Stretch, 2, 2, true)[3], 0);
    }

    /// A new rectangle has the picture's shape, covers what it was asked to,
    /// and is held larger-first, as the shipped rows are.
    #[test]
    fn a_new_rectangle_has_the_pictures_shape() {
        let [left, right, top, bottom] = fit_rect([-1000.0, -500.0, 1000.0, 500.0]);
        assert!(left > right && top > bottom);
        let ratio = (left - right) / (top - bottom);
        assert!((ratio - WIDTH as f32 / HEIGHT as f32).abs() < 1e-3, "{ratio}");
        assert!(top >= 1000.0 && bottom <= -1000.0 && left >= 500.0 && right <= -500.0);
    }

    /// The zone grid takes the commonest id in each half tile, and reads
    /// back from its file. A cell's row is from x and its column from y.
    #[test]
    fn the_zone_grid_takes_the_commonest_area_in_each_half_tile() {
        // The cell of the map's middle, the world origin, is (64, 64).
        assert_eq!(grid_cell(-1.0, -1.0), Some(64 * GRID_SIDE + 64));
        // North of it is a lower row; west of it a lower column.
        assert_eq!(grid_cell(100.0, -1.0), Some(63 * GRID_SIDE + 64));
        assert_eq!(grid_cell(-1.0, 100.0), Some(64 * GRID_SIDE + 63));
        assert_eq!(grid_cell(MAP_YARDS, 0.0), None);
        let chunks = [(-10.0, -10.0, 12), (-20.0, -20.0, 12), (-30.0, -30.0, 40), (100.0, -10.0, 7)];
        let grid = zone_grid(chunks);
        assert_eq!(grid[64 * GRID_SIDE + 64], 12);
        assert_eq!(grid[63 * GRID_SIDE + 64], 7);
        assert_eq!(grid.iter().filter(|id| **id != 0).count(), 2);
        assert_eq!(read_zmp(&zmp_bytes(&grid)), Some(grid));
        assert_eq!(read_zmp(&[0; 10]), None);
    }

    /// A zone's sheet is lit inside the zone and in the top part of the
    /// sheet only; a continent's is lit over the tiles it has.
    #[test]
    fn a_highlight_sheet_lights_the_zone_and_nothing_else() {
        let area = MapArea {
            id: 1,
            map: 0,
            area: 12,
            directory: "Test".to_string(),
            left: 1000.0,
            right: -500.0,
            top: 1000.0,
            bottom: 0.0,
        };
        // The zone is the rectangle's west half (y above 250).
        let sheet = zone_highlight(&area, 85.0 / 128.0, |_, y| y > 250.0);
        assert_eq!(sheet.len(), (ZONE_SHEET * ZONE_SHEET * 4) as usize);
        let lit = |s: &[u8], side: u32, x: u32, y: u32| s[((y * side + x) * 4) as usize + 1];
        assert!(lit(&sheet, ZONE_SHEET, 10, 10) > 40, "west is lit");
        assert_eq!(lit(&sheet, ZONE_SHEET, 120, 10), 0, "east is not");
        assert_eq!(lit(&sheet, ZONE_SHEET, 10, 120), 0, "below the fraction is not");
        // Two tiles across, one down: the top half of the sheet, and only the
        // first tile is there.
        let continent = continent_highlight([30, 31, 40, 40], |col, _| col == 30);
        assert!(lit(&continent, CONTINENT_SHEET, 10, 10) > 40);
        assert_eq!(lit(&continent, CONTINENT_SHEET, 200, 10), 0);
        assert_eq!(lit(&continent, CONTINENT_SHEET, 10, 200), 0);
    }

    /// Ground with no minimap tile is black and counted as missing.
    #[test]
    fn ground_with_no_minimap_tile_is_black() {
        let north = MAP_ORIGIN - 10.0 * TILE_SIZE;
        let west = MAP_ORIGIN - 10.0 * TILE_SIZE;
        let area = MapArea {
            id: 1,
            map: 0,
            area: 12,
            directory: "Test".to_string(),
            left: west,
            right: west - 2.0 * TILE_SIZE,
            top: north,
            bottom: north - TILE_SIZE,
        };
        let white = vec![255u8; (TILE_SIDE * TILE_SIDE * 4) as usize];
        let drawn = from_minimap(&area, |col, _| (col == 10).then(|| white.clone()), |_| {});
        assert_eq!((drawn.drawn, drawn.missing), (1, 1));
        let at = |u: u32| drawn.picture[((HEIGHT / 2 * WIDTH + u) * 4) as usize];
        assert_eq!(at(10), 255, "the west tile is drawn");
        assert_eq!(at(WIDTH - 10), 0, "the east tile is missing");
    }
}
