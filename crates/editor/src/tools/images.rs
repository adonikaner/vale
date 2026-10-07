//! Ground as picture files: a height map or a blend map of the selected
//! tiles written to a PNG, and any PNG read back onto them.
//!
//! What a pixel means is `vale_edit::ops::image`. This file is the half that
//! touches the disk and the session.
//!
//! ## Export
//!
//! The map window's Export heights… and Export blends… ask where to save, and
//! write one picture of the whole selection: the block of tiles from its
//! north-west tile to its south-east one, tiles in the block that are not
//! selected or do not exist left black. A height map is 16-bit grey and
//! carries the heights black and white stand for in a text chunk
//! ([`RANGE_KEY`]), so the file is all there is.
//!
//! ## Import
//!
//! Import… asks for any PNG: 16-bit or 8-bit grey, grey saved as colour, or
//! colour. [`read`] decodes it into a [`Pending`] import, which the map window
//! shows before anything changes: what the picture is used as, the tiles it
//! lands on, and for a height map the two heights black and white stand for.
//! Those come from the file when it carries them and otherwise from the
//! selected tiles' own lowest and highest vertex, since a picture drawn in a
//! paint program says only where is higher. A picture of another size than
//! the block is scaled to fit it.
//!
//! An import is one undo entry. With Objects follow the ground on, what
//! stands on a tile is carried by as much as the ground under it moved
//! (`super::terrain::carry`). Only open tiles are written, which are the ones
//! near the camera; the window says how many of the selection that is.

use crate::session::EditSession;
use std::path::{Path, PathBuf};
use vale_client::assets::GameAssets;
use vale_edit::adt::AdtFile;
use vale_edit::ops::image::{self, HeightMap, BLEND_SIDE, HEIGHT_SIDE};

/// The PNG text keyword a height map's range is written under, as
/// `low high` in yards.
pub const RANGE_KEY: &str = "Height range";

/// What a picture is imported as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Heights,
    Blends,
}

/// The block a selection covers: its north-west tile, and how many tiles
/// across and down. `None` for no tiles.
pub fn block(tiles: &[(u32, u32)]) -> Option<((u32, u32), (u32, u32))> {
    let x0 = tiles.iter().map(|at| at.0).min()?;
    let y0 = tiles.iter().map(|at| at.1).min()?;
    let x1 = tiles.iter().map(|at| at.0).max()?;
    let y1 = tiles.iter().map(|at| at.1).max()?;
    Some(((x0, y0), (x1 - x0 + 1, y1 - y0 + 1)))
}

/// The pixels a block's picture is, across and down.
pub fn size_of(kind: Kind, (wide, tall): (u32, u32)) -> (usize, usize) {
    match kind {
        Kind::Heights => (wide as usize * (HEIGHT_SIDE - 1) + 1, tall as usize * (HEIGHT_SIDE - 1) + 1),
        Kind::Blends => (wide as usize * BLEND_SIDE, tall as usize * BLEND_SIDE),
    }
}

/// The block's tiles in words: `32, 48`, or `32, 48 to 33, 49`.
pub fn block_words(((x, y), (wide, tall)): ((u32, u32), (u32, u32))) -> String {
    match (wide, tall) {
        (1, 1) => format!("{x}, {y}"),
        _ => format!("{x}, {y} to {}, {}", x + wide - 1, y + tall - 1),
    }
}

/// The file name an export suggests.
pub fn suggested_name(map: &str, tiles: &[(u32, u32)], kind: Kind) -> String {
    let what = match kind {
        Kind::Heights => "heights",
        Kind::Blends => "blends",
    };
    match block(tiles) {
        Some(((x, y), (1, 1))) => format!("{map} {x}_{y} {what}.png"),
        Some(((x, y), (wide, tall))) => {
            format!("{map} {x}_{y} to {}_{} {what}.png", x + wide - 1, y + tall - 1)
        }
        None => format!("{map} {what}.png"),
    }
}

fn said(path: &Path, error: impl std::fmt::Display) -> String {
    format!("{}: {error}", path.display())
}

/// A tile as the session has it: open and edited, or read from the project
/// or the archives.
fn tile_of(session: &EditSession, assets: &GameAssets, at: (u32, u32)) -> Option<AdtFile> {
    match session.tiles.get(&at) {
        Some(tile) => Some(tile.clone()),
        None => AdtFile::parse(&session.tile_bytes(assets, at)?).ok(),
    }
}

/// The lowest and highest vertex over the open tiles of `tiles`, which is
/// the range a picture with none of its own is imported over.
pub fn open_range(session: &EditSession, tiles: &[(u32, u32)]) -> Option<(f32, f32)> {
    tiles
        .iter()
        .filter_map(|at| image::height_range(session.tiles.get(at)?))
        .reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)))
}

/// Write the selection's picture to `path`. Returns the status line.
pub fn export(
    session: &EditSession,
    assets: &GameAssets,
    tiles: &[(u32, u32)],
    kind: Kind,
    path: &Path,
) -> Result<String, String> {
    let Some((origin, span)) = block(tiles) else {
        return Err("no tiles selected".into());
    };
    let found: Vec<((u32, u32), AdtFile)> = tiles
        .iter()
        .filter_map(|&at| Some((at, tile_of(session, assets, at)?)))
        .collect();
    if found.is_empty() {
        return Err("none of the selected tiles exists".into());
    }
    let (width, height) = size_of(kind, span);
    let offset = |at: (u32, u32), side: usize| {
        (
            (at.0 - origin.0) as usize * side,
            (at.1 - origin.1) as usize * side,
        )
    };
    match kind {
        Kind::Heights => {
            let range = found
                .iter()
                .filter_map(|(_, tile)| image::height_range(tile))
                .reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)))
                .ok_or("the selected tiles are missing chunks")?;
            let (low, high) = image::widened(range);
            let mut pixels = vec![0u16; width * height];
            for (at, tile) in &found {
                let Some(map) = image::export_heights_over(tile, low, high) else {
                    continue;
                };
                let at = offset(*at, HEIGHT_SIDE - 1);
                image::place(&mut pixels, width, 1, HEIGHT_SIDE, at, &map.pixels);
            }
            let bytes: Vec<u8> = pixels.iter().flat_map(|pixel| pixel.to_be_bytes()).collect();
            write_png(path, (width, height), Kind::Heights, &bytes, Some((low, high)))?;
            Ok(format!(
                "wrote the heights of {} to {}: black {low:.1} yd, white {high:.1} yd",
                block_words((origin, span)),
                path.display()
            ))
        }
        Kind::Blends => {
            let mut rgb = vec![0u8; width * height * 3];
            for (at, tile) in &found {
                let at = offset(*at, BLEND_SIDE);
                image::place(&mut rgb, width, 3, BLEND_SIDE, at, &image::export_blend(tile));
            }
            write_png(path, (width, height), Kind::Blends, &rgb, None)?;
            Ok(format!(
                "wrote the blends of {} to {}",
                block_words((origin, span)),
                path.display()
            ))
        }
    }
}

/// Encode a picture: 16-bit grey from big-endian pairs for heights, with
/// the range as a text chunk, or 8-bit RGB for blends.
fn write_png(
    path: &Path,
    (width, height): (usize, usize),
    kind: Kind,
    bytes: &[u8],
    range: Option<(f32, f32)>,
) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| said(path, e))?;
    let mut encoder =
        png::Encoder::new(std::io::BufWriter::new(file), width as u32, height as u32);
    match kind {
        Kind::Heights => {
            encoder.set_color(png::ColorType::Grayscale);
            encoder.set_depth(png::BitDepth::Sixteen);
        }
        Kind::Blends => {
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
        }
    }
    if let Some((low, high)) = range {
        encoder
            .add_text_chunk(RANGE_KEY.to_string(), format!("{low} {high}"))
            .map_err(|e| said(path, e))?;
    }
    let mut writer = encoder.write_header().map_err(|e| said(path, e))?;
    writer.write_image_data(bytes).map_err(|e| said(path, e))
}

/// A picture chosen for import and not written yet: what the map window's
/// dialog shows, and what [`import`] writes.
#[derive(Debug, Clone)]
pub struct Pending {
    pub path: PathBuf,
    /// Pixels across and down.
    pub size: (usize, usize),
    /// What the picture looks like it is, and what it is to be used as.
    pub found: Kind,
    pub kind: Kind,
    /// The heights black and white stand for, and whether the file said so.
    pub low: f32,
    pub high: f32,
    pub range_in_file: bool,
    /// Grey, 0 to 1 a pixel.
    grey: Vec<f32>,
    /// Red, green and blue, 0 to 255 a channel.
    rgb: Vec<f32>,
}

impl Pending {
    /// The file's name, for the dialog's heading.
    pub fn name(&self) -> String {
        self.path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}

/// Decode any PNG for import: grey or colour, 8 or 16 bits, with or without
/// alpha, which is dropped. A colour picture whose three channels agree is
/// grey saved as colour and is taken as a height map.
pub fn read(path: &Path) -> Result<Pending, String> {
    let file = std::fs::File::open(path).map_err(|e| said(path, e))?;
    let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().map_err(|e| said(path, e))?;
    let range = reader
        .info()
        .uncompressed_latin1_text
        .iter()
        .find(|chunk| chunk.keyword == RANGE_KEY)
        .and_then(|chunk| {
            let mut numbers = chunk.text.split_whitespace().map(str::parse::<f32>);
            match (numbers.next(), numbers.next()) {
                (Some(Ok(low)), Some(Ok(high))) if high > low => Some((low, high)),
                _ => None,
            }
        });
    let size = reader
        .output_buffer_size()
        .ok_or_else(|| said(path, "too large to read"))?;
    let mut bytes = vec![0u8; size];
    let frame = reader.next_frame(&mut bytes).map_err(|e| said(path, e))?;
    bytes.truncate(frame.buffer_size());
    let (width, height) = (frame.width as usize, frame.height as usize);
    let channels = match frame.color_type {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => return Err(said(path, "indexed-colour PNG that could not be expanded")),
    };
    // Every sample as 0 to 1.
    let samples: Vec<f32> = match frame.bit_depth {
        png::BitDepth::Sixteen => bytes
            .chunks_exact(2)
            .map(|pair| f32::from(u16::from_be_bytes([pair[0], pair[1]])) / 65535.0)
            .collect(),
        _ => bytes.iter().map(|&byte| f32::from(byte) / 255.0).collect(),
    };
    let pixels = samples.chunks_exact(channels);
    let (grey, rgb): (Vec<f32>, Vec<[f32; 3]>) = match channels {
        1 | 2 => pixels.map(|p| (p[0], [p[0] * 255.0; 3])).unzip(),
        _ => pixels
            .map(|p| ((p[0] + p[1] + p[2]) / 3.0, [p[0] * 255.0, p[1] * 255.0, p[2] * 255.0]))
            .unzip(),
    };
    let coloured = channels >= 3
        && rgb
            .iter()
            .any(|[r, g, b]| (r - g).abs() > 1.0 || (g - b).abs() > 1.0);
    let found = match coloured {
        true => Kind::Blends,
        false => Kind::Heights,
    };
    let (low, high) = range.unwrap_or((0.0, 100.0));
    Ok(Pending {
        path: path.to_path_buf(),
        size: (width, height),
        found,
        kind: found,
        low,
        high,
        range_in_file: range.is_some(),
        grey,
        rgb: rgb.into_iter().flatten().collect(),
    })
}

/// Write a pending picture onto the open tiles of `tiles`, as one undo
/// entry. Returns the status line.
pub fn import(
    session: &mut EditSession,
    pending: &Pending,
    tiles: &[(u32, u32)],
    objects_follow: bool,
) -> String {
    let Some((origin, span)) = block(tiles) else {
        return "no tiles selected".into();
    };
    if pending.kind == Kind::Heights && pending.high <= pending.low {
        return "white must be higher than black".into();
    }
    let (width, height) = size_of(pending.kind, span);
    // The picture fitted to the block, then cut into tiles.
    let fitted = match pending.kind {
        Kind::Heights => image::resample(&pending.grey, 1, pending.size, (width, height)),
        Kind::Blends => image::resample(&pending.rgb, 3, pending.size, (width, height)),
    };
    let label = match pending.kind {
        Kind::Heights => "Import height map",
        Kind::Blends => "Import blend map",
    };
    let (mut changed, mut chunks, mut open) = (0usize, 0usize, 0usize);
    let mut standing = Vec::new();
    let mut touched = Vec::new();
    session.history.begin(label);
    for &at in tiles {
        if !session.tiles.contains_key(&at) {
            continue;
        }
        open += 1;
        let key = session.key(at);
        let Some(tile) = session.tiles.get_mut(&at) else {
            continue;
        };
        let mut edits = match pending.kind {
            Kind::Heights => {
                let at_pixel = (
                    (at.0 - origin.0) as usize * (HEIGHT_SIDE - 1),
                    (at.1 - origin.1) as usize * (HEIGHT_SIDE - 1),
                );
                let square = image::window(&fitted, width, 1, HEIGHT_SIDE, at_pixel);
                let map = HeightMap {
                    low: pending.low,
                    high: pending.high,
                    pixels: square
                        .iter()
                        .map(|v| (v.clamp(0.0, 1.0) * 65535.0).round() as u16)
                        .collect(),
                };
                if objects_follow {
                    standing.push((at, vale_edit::ops::follow::standing(tile)));
                }
                image::import_heights(tile, &map)
            }
            Kind::Blends => {
                let at_pixel = (
                    (at.0 - origin.0) as usize * BLEND_SIDE,
                    (at.1 - origin.1) as usize * BLEND_SIDE,
                );
                let square = image::window(&fitted, width, 3, BLEND_SIDE, at_pixel);
                let rgb: Vec<u8> = square.iter().map(|v| v.round().clamp(0.0, 255.0) as u8).collect();
                image::import_blend(tile, &rgb)
            }
        };
        session.hold_locked(at, &mut edits);
        if edits.is_empty() {
            continue;
        }
        changed += 1;
        chunks += edits
            .iter()
            .filter(|edit| {
                matches!(
                    edit,
                    vale_edit::ops::Edit::Heights { .. } | vale_edit::ops::Edit::Paint { .. }
                )
            })
            .count();
        session.history.record(&key, edits);
        touched.push(at);
    }
    let carried = super::terrain::carry(session, standing);
    session.history.end();
    // The whole tile is read again: an import can move every vertex and
    // every blend of it, which is more than the live paths patch.
    for at in touched {
        session.publish(at);
        session.stale.insert(at);
        session.unsaved.insert(at);
    }
    let mut line = match (open, changed) {
        (0, _) => "none of the selected tiles is open: fly to them first".to_string(),
        (_, 0) => "the image matches the selected tiles: nothing changed".to_string(),
        (_, n) => format!(
            "imported {} onto {n} tile{}: {chunks} chunk change{}",
            pending.name(),
            if n == 1 { "" } else { "s" },
            if chunks == 1 { "" } else { "s" }
        ),
    };
    if carried > 0 {
        line.push_str(&format!(", {carried} doodad/WMO placements moved with the ground"));
    }
    if open > 0 && open < tiles.len() {
        line.push_str(&format!("; {} selected tiles were not open", tiles.len() - open));
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("vale-images-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A height map carries its range inside the file and reads back as the
    /// same sixteen bits.
    #[test]
    fn a_height_map_carries_its_range_and_reads_back() {
        let dir = scratch("height");
        let path = dir.join("Azeroth 32_48 heights.png");
        let pixels: Vec<u16> = (0..HEIGHT_SIDE * HEIGHT_SIDE).map(|i| (i * 7 % 65536) as u16).collect();
        let bytes: Vec<u8> = pixels.iter().flat_map(|p| p.to_be_bytes()).collect();
        write_png(&path, (HEIGHT_SIDE, HEIGHT_SIDE), Kind::Heights, &bytes, Some((18.25, 170.5))).unwrap();
        let read = read(&path).unwrap();
        assert_eq!((read.found, read.range_in_file), (Kind::Heights, true));
        assert_eq!((read.low, read.high), (18.25, 170.5));
        let back: Vec<u16> = read.grey.iter().map(|v| (v * 65535.0).round() as u16).collect();
        assert_eq!(back, pixels);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A picture from elsewhere: grey saved as colour is a height map with no
    /// range of its own, and a coloured one is a blend map.
    #[test]
    fn a_picture_from_elsewhere_is_told_apart_by_its_colours() {
        let dir = scratch("elsewhere");
        let grey = dir.join("grey.png");
        let rgb: Vec<u8> = (0..64 * 64).flat_map(|i| [(i % 256) as u8; 3]).collect();
        write_png(&grey, (64, 64), Kind::Blends, &rgb, None).unwrap();
        let read_grey = read(&grey).unwrap();
        assert_eq!((read_grey.found, read_grey.range_in_file), (Kind::Heights, false));
        assert_eq!(read_grey.size, (64, 64));

        let colour = dir.join("colour.png");
        let rgb: Vec<u8> = (0..64 * 64).flat_map(|i| [(i % 256) as u8, 0, 255]).collect();
        write_png(&colour, (64, 64), Kind::Blends, &rgb, None).unwrap();
        assert_eq!(read(&colour).unwrap().found, Kind::Blends);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A block's size and name follow the selection's corners.
    #[test]
    fn a_selection_is_a_block_from_its_corners() {
        let tiles = [(33, 49), (32, 48), (32, 49)];
        assert_eq!(block(&tiles), Some(((32, 48), (2, 2))));
        assert_eq!(size_of(Kind::Heights, (2, 2)), (513, 513));
        assert_eq!(size_of(Kind::Blends, (2, 1)), (2048, 1024));
        assert_eq!(block_words(((32, 48), (2, 2))), "32, 48 to 33, 49");
        assert_eq!(suggested_name("Azeroth", &[(32, 48)], Kind::Heights), "Azeroth 32_48 heights.png");
    }
}
